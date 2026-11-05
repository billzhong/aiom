use flate2::{Compression, bufread::MultiGzDecoder, write::GzEncoder};
use serde::{Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};

/// Error returned by [`encode`] and [`decode`].
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// Compressing failed, or the input is not a valid gzip stream.
    #[error("gzip error: {0}")]
    Compression(#[from] io::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("decompressed message exceeds {limit} bytes")]
    TooLarge { limit: usize },
}

/// Serializes `value` as gzip-compressed JSON.
pub fn encode<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, CodecError> {
    // Serialize up front: streaming serde_json into the encoder issues one tiny write per token.
    Ok(deflate(&serde_json::to_vec(value)?)?)
}

/// Compresses `bytes` into one gzip member.
fn deflate(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::with_capacity(bytes.len() / 2), Compression::default());
    encoder.write_all(bytes)?;
    encoder.finish()
}

/// Deserializes gzip-compressed JSON, refusing to inflate more than `max_len` bytes so that a
/// small malicious payload cannot exhaust memory.
pub fn decode<T: DeserializeOwned>(bytes: &[u8], max_len: usize) -> Result<T, CodecError> {
    Ok(serde_json::from_slice(&inflate(bytes, max_len)?)?)
}

/// Decompresses `bytes`, refusing to inflate more than `max_len` bytes. The gzip members of a
/// multi-member stream are joined, while anything else after the first member is an error.
pub(crate) fn inflate(bytes: &[u8], max_len: usize) -> Result<Vec<u8>, CodecError> {
    // JSON inflates to a few times its compressed size.
    let mut json = Vec::with_capacity(bytes.len().saturating_mul(4).min(max_len));
    let limit = u64::try_from(max_len).unwrap_or(u64::MAX).saturating_add(1);
    MultiGzDecoder::new(bytes)
        .take(limit)
        .read_to_end(&mut json)?;
    if json.len() > max_len {
        return Err(CodecError::TooLarge { limit: max_len });
    }
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeSnapshot, Stats};

    fn snapshot(name: &str) -> NodeSnapshot {
        NodeSnapshot {
            timestamp: 42,
            name: name.into(),
            host: Default::default(),
            stats: Stats {
                cpu_usage: 3.5,
                ..Default::default()
            },
            geoip: "JP".into(),
        }
    }

    #[test]
    fn round_trips() {
        let nodes = vec![snapshot("a"), snapshot("b")];
        let encoded = encode(&nodes).unwrap();
        assert_eq!(&encoded[..2], &[0x1f, 0x8b], "output must be gzip");
        let decoded: Vec<NodeSnapshot> = decode(&encoded, 1 << 20).unwrap();
        assert_eq!(decoded, nodes);
    }

    #[test]
    fn accepts_payload_exactly_at_limit() {
        let encoded = encode("abc").unwrap();
        let decoded: String = decode(&encoded, "\"abc\"".len()).unwrap();
        assert_eq!(decoded, "abc");
    }

    #[test]
    fn rejects_oversized_payload() {
        let encoded = encode(&"x".repeat(10_000)).unwrap();
        let err = decode::<String>(&encoded, 1_000).unwrap_err();
        assert!(
            matches!(err, CodecError::TooLarge { limit: 1_000 }),
            "{err}"
        );
    }

    #[test]
    fn rejects_non_gzip_input() {
        let err = decode::<String>(br#""plain json""#, 1_000).unwrap_err();
        assert!(matches!(err, CodecError::Compression(_)), "{err}");
    }

    #[test]
    fn joins_the_members_of_a_stream() {
        let stream = [deflate(b"\"ab").unwrap(), deflate(b"c\"").unwrap()].concat();
        assert_eq!(decode::<String>(&stream, 1_000).unwrap(), "abc");
    }

    #[test]
    fn rejects_data_after_the_message() {
        let trailing_garbage = [encode("abc").unwrap(), b"junk".to_vec()].concat();
        let err = decode::<String>(&trailing_garbage, 1_000).unwrap_err();
        assert!(matches!(err, CodecError::Compression(_)), "{err}");

        let two_messages = [encode("abc").unwrap(), encode("def").unwrap()].concat();
        let err = decode::<String>(&two_messages, 1_000).unwrap_err();
        assert!(matches!(err, CodecError::Json(_)), "{err}");
    }

    #[test]
    fn rejects_invalid_json() {
        let err = decode::<String>(&deflate(b"{not json").unwrap(), 1_000).unwrap_err();
        assert!(matches!(err, CodecError::Json(_)), "{err}");
    }
}

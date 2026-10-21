use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde::{Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};

/// Error returned by [`encode`] and [`decode`].
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("invalid gzip stream: {0}")]
    Compression(#[from] io::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("decompressed message exceeds {limit} bytes")]
    TooLarge { limit: usize },
}

/// Serializes `value` as gzip-compressed JSON.
pub fn encode<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, CodecError> {
    // Serialize up front: streaming serde_json into the encoder issues one tiny write per token.
    let json = serde_json::to_vec(value)?;
    let mut encoder = GzEncoder::new(Vec::with_capacity(json.len() / 2), Compression::default());
    encoder.write_all(&json)?;
    Ok(encoder.finish()?)
}

/// Deserializes gzip-compressed JSON, refusing to inflate more than `max_len` bytes so that a
/// small malicious payload cannot exhaust memory.
pub fn decode<T: DeserializeOwned>(bytes: &[u8], max_len: usize) -> Result<T, CodecError> {
    let mut json = Vec::new();
    let limit = u64::try_from(max_len).unwrap_or(u64::MAX).saturating_add(1);
    GzDecoder::new(bytes).take(limit).read_to_end(&mut json)?;
    if json.len() > max_len {
        return Err(CodecError::TooLarge { limit: max_len });
    }
    Ok(serde_json::from_slice(&json)?)
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
    fn rejects_invalid_json() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"{not json").unwrap();
        let err = decode::<String>(&encoder.finish().unwrap(), 1_000).unwrap_err();
        assert!(matches!(err, CodecError::Json(_)), "{err}");
    }
}

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::fmt;
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// The token clients must send as `Authorization: Bearer <token>`.
#[derive(Clone)]
pub(crate) struct BearerToken(Arc<[u8]>);

impl BearerToken {
    pub(crate) fn new(token: &str) -> Self {
        Self(token.as_bytes().into())
    }

    fn matches(&self, headers: &HeaderMap) -> bool {
        let Some((scheme, token)) = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split_once(' '))
        else {
            return false;
        };
        // Constant-time, so response timing does not reveal how much of the token matched.
        scheme.eq_ignore_ascii_case("bearer") && bool::from(token.as_bytes().ct_eq(&self.0))
    }
}

impl fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BearerToken(..)")
    }
}

/// Middleware rejecting requests without the expected bearer token.
pub(crate) async fn require_bearer(
    State(expected): State<BearerToken>,
    request: Request,
    next: Next,
) -> Response {
    if expected.matches(request.headers()) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"))],
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(authorization: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, authorization.parse().unwrap());
        headers
    }

    #[test]
    fn accepts_the_exact_token() {
        let token = BearerToken::new("s3cret");
        assert!(token.matches(&headers("Bearer s3cret")));
        assert!(token.matches(&headers("bearer s3cret")));
    }

    #[test]
    fn rejects_everything_else() {
        let token = BearerToken::new("s3cret");
        assert!(!token.matches(&HeaderMap::new()));
        assert!(!token.matches(&headers("Bearer")));
        assert!(!token.matches(&headers("Bearer ")));
        assert!(!token.matches(&headers("Bearer s3cre")));
        assert!(!token.matches(&headers("Bearer s3cret2")));
        assert!(!token.matches(&headers("Basic s3cret")));
        assert!(!token.matches(&headers("s3cret")));
    }

    #[test]
    fn debug_output_hides_the_token() {
        assert_eq!(
            format!("{:?}", BearerToken::new("s3cret")),
            "BearerToken(..)"
        );
    }
}

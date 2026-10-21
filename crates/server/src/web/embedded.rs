//! The web UI built into the binary.

use super::INDEX;
use axum::Router;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;
use std::path::{Component, Path};

/// The `web/dist` of the build, or a fixture in tests. Debug builds read it from disk instead,
/// so a rebuilt web UI shows up without rebuilding the server.
#[derive(RustEmbed)]
#[cfg_attr(not(test), folder = "../../web/dist")]
#[cfg_attr(test, folder = "fixtures/web")]
struct Dashboard;

/// Whether the web UI has an `index.html`.
#[cfg(feature = "embed-web")]
pub(super) fn has_index() -> bool {
    Dashboard::get(INDEX).is_some()
}

/// Serves the files of the web UI, with `index.html` for directories.
pub(super) fn router() -> Router {
    Router::new().fallback_service(get(serve))
}

/// Answers with the file of the web UI at the path of `uri`. Unlike `ServeDir`, does not
/// percent-decode the path: the build names files with characters that need no encoding.
async fn serve(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    // rust-embed normalizes backslashes and permits some escaping symlinks in debug builds.
    // Accept only relative paths without parent components before its filesystem lookup.
    if path.contains('\\')
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let file = if path.is_empty() || path.ends_with('/') {
        Dashboard::get(&format!("{path}{INDEX}"))
    } else {
        Dashboard::get(path)
    };
    let Some(file) = file else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let etag = etag(&file.metadata.sha256_hash());
    if is_cached(&headers, &etag) {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
    }
    (
        [
            (header::CONTENT_TYPE, file.metadata.mimetype()),
            (header::ETAG, etag.as_str()),
        ],
        file.data,
    )
        .into_response()
}

/// The entity tag of content hashed to `hash`. Weak, as compression changes the bytes sent but
/// not the tag.
fn etag(hash: &[u8; 32]) -> String {
    let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("W/\"{hex}\"")
}

/// Whether the `If-None-Match` of `headers` lists `etag`, compared weakly as RFC 9110 asks.
fn is_cached(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .any(|tag| tag == "*" || opaque(tag) == opaque(etag))
}

/// The entity tag `tag` without its weakness indicator.
fn opaque(tag: &str) -> &str {
    tag.strip_prefix("W/").unwrap_or(tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    async fn request(method: Method, path: &str, if_none_match: Option<&str>) -> Response {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(tag) = if_none_match {
            request = request.header(header::IF_NONE_MATCH, tag);
        }
        let request = request.body(Body::empty()).unwrap();
        router().oneshot(request).await.unwrap()
    }

    async fn get(path: &str) -> Response {
        request(Method::GET, path, None).await
    }

    async fn body(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn serves_files_with_their_type() {
        for (path, content_type, content) in [
            (
                "/",
                "text/html",
                include_str!("../../fixtures/web/index.html"),
            ),
            (
                "/assets/index-abc123.js",
                "text/javascript",
                include_str!("../../fixtures/web/assets/index-abc123.js"),
            ),
        ] {
            let response = get(path).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
            assert_eq!(body(response).await, content);
        }

        for path in ["/missing.js", "/assets/", "/../Cargo.toml"] {
            assert_eq!(get(path).await.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn rejects_unsafe_paths() {
        for path in [
            "/../web/index.html",
            "/assets/../../web/index.html",
            r"/assets\index-abc123.js",
            "/%2e%2e/web/index.html",
            "/assets%5cindex-abc123.js",
        ] {
            assert_eq!(get(path).await.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[cfg(all(unix, debug_assertions))]
    #[tokio::test]
    async fn rejects_traversal_to_symlinks_outside_dashboard() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("secret");
        let link = dir.path().join("link");
        std::fs::write(&secret, "outside the dashboard").unwrap();
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        // Climb from fixtures/web to the filesystem root, then request the external link.
        let depth = Path::new(env!("CARGO_MANIFEST_DIR")).components().count() + 2;
        for path in [
            format!("/{}{}", "../".repeat(depth), link.display()),
            format!("/\\{}", link.display()),
        ] {
            assert_eq!(get(&path).await.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn revalidates_by_entity_tag() {
        let page = get("/").await;
        let etag = page.headers()[header::ETAG].to_str().unwrap().to_owned();
        assert!(etag.starts_with("W/\""), "{etag}");
        assert_eq!(etag.len(), 68);

        for tag in [
            etag.clone(),
            opaque(&etag).to_owned(),
            format!("\"other\", {etag}"),
            "*".to_owned(),
        ] {
            let cached = request(Method::GET, "/", Some(&tag)).await;
            assert_eq!(cached.status(), StatusCode::NOT_MODIFIED, "{tag}");
            assert_eq!(cached.headers()[header::ETAG], etag.as_str());
            assert!(body(cached).await.is_empty());
        }

        let stale = request(Method::GET, "/", Some("\"other\"")).await;
        assert_eq!(stale.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn answers_head_but_no_other_methods() {
        let head = request(Method::HEAD, "/", None).await;
        assert_eq!(head.status(), StatusCode::OK);
        assert!(body(head).await.is_empty());

        let post = request(Method::POST, "/", None).await;
        assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}

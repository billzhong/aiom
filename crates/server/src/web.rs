//! The dashboard, served from the directory holding the built web UI.

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use std::path::Path;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;

/// Where the build puts its assets, whose file names contain a hash of their content.
const ASSETS_PREFIX: &str = "/assets/";

/// Serves the files in `dir`, with `index.html` for directories.
pub(crate) fn router(dir: &Path) -> Router {
    Router::new()
        .fallback_service(ServeDir::new(dir))
        .layer(middleware::from_fn(cache_control))
        .layer(CompressionLayer::new())
}

/// Lets browsers keep assets for good, while revalidating everything else, such as
/// `index.html`, so that an upgraded dashboard takes effect on the next load.
async fn cache_control(request: Request, next: Next) -> Response {
    let immutable = request.uri().path().starts_with(ASSETS_PREFIX);
    let mut response = next.run(request).await;
    if response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED {
        let value = if immutable {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    }
    response
}

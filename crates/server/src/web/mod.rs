//! The dashboard, served from the directory holding the built web UI, or from the binary.

#[cfg(any(feature = "embed-web", test))]
mod embedded;

use anyhow::ensure;
use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use std::fmt;
use std::path::PathBuf;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;

/// Where the build puts its assets, whose file names contain a hash of their content.
const ASSETS_PREFIX: &str = "/assets/";

const INDEX: &str = "index.html";

/// Where the built web UI is served from.
#[derive(Debug)]
pub(crate) enum Source {
    /// A directory on disk.
    Dir(PathBuf),
    /// The `web/dist` of the build, built into the binary.
    #[cfg(feature = "embed-web")]
    Embedded,
}

impl Source {
    /// The directory `dir` if set, otherwise the web UI built into the binary, if any.
    pub(crate) fn new(dir: Option<&str>) -> anyhow::Result<Option<Self>> {
        let source = match dir.map(PathBuf::from) {
            Some(dir) => {
                ensure!(
                    dir.join(INDEX).is_file(),
                    "web directory {dir:?} has no index.html"
                );
                Self::Dir(dir)
            }
            #[cfg(feature = "embed-web")]
            None => {
                ensure!(
                    embedded::has_index(),
                    "the web UI built into the binary has no index.html"
                );
                Self::Embedded
            }
            #[cfg(not(feature = "embed-web"))]
            None => return Ok(None),
        };
        Ok(Some(source))
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dir(dir) => dir.display().fmt(f),
            #[cfg(feature = "embed-web")]
            Self::Embedded => f.write_str("embedded"),
        }
    }
}

/// Serves the files of `source`, with `index.html` for directories.
pub(crate) fn router(source: &Source) -> Router {
    let files = match source {
        Source::Dir(dir) => Router::new().fallback_service(ServeDir::new(dir)),
        #[cfg(feature = "embed-web")]
        Source::Embedded => embedded::router(),
    };
    files
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

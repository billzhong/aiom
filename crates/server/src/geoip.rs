//! IP geolocation through ipinfo.io.

use aiom_protocol::is_public;
use anyhow::{Context, bail};
use reqwest::redirect::Policy;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

const IPINFO_URL: &str = "https://ipinfo.io";
const TIMEOUT: Duration = Duration::from_secs(3);
/// How long the country found for an address is reused.
const FOUND_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// How long to wait before looking up an address again after a failure.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);
/// Cache size above which expired entries are dropped.
const PRUNE_ABOVE: usize = 1024;

/// Resolves IP addresses to ISO 3166-1 alpha-2 country codes, caching the results.
#[derive(Debug, Clone)]
pub struct GeoIp {
    inner: Option<Arc<Ipinfo>>,
}

#[derive(Debug)]
struct Ipinfo {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
    cache: Mutex<HashMap<IpAddr, Lookup>>,
}

/// A lookup of one address, shared by everyone asking for it: concurrent requests for the
/// same address wait for one lookup.
#[derive(Debug, Clone)]
struct Lookup {
    started: Instant,
    country: Arc<OnceCell<Option<String>>>,
}

impl Lookup {
    fn new(now: Instant) -> Self {
        Self {
            started: now,
            country: Arc::default(),
        }
    }

    /// Whether the result may still be used; lookups in flight always may.
    fn is_fresh(&self, now: Instant) -> bool {
        let ttl = match self.country.get() {
            None => return true,
            Some(Some(_)) => FOUND_TTL,
            Some(None) => RETRY_AFTER,
        };
        now < self.started + ttl
    }
}

impl GeoIp {
    /// Looks addresses up on ipinfo.io, authenticating with `token` when given.
    pub fn ipinfo(token: Option<String>) -> anyhow::Result<Self> {
        Self::with_base_url(IPINFO_URL, token)
    }

    /// Like [`GeoIp::ipinfo`], against an ipinfo-compatible service at `base_url`.
    pub fn with_base_url(base_url: &str, token: Option<String>) -> anyhow::Result<Self> {
        crate::install_crypto_provider();
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(Policy::none())
            .user_agent(concat!("aiom-server/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            inner: Some(Arc::new(Ipinfo {
                http,
                base_url: base_url.trim_end_matches('/').to_owned(),
                token,
                cache: Mutex::default(),
            })),
        })
    }

    /// A resolver that never finds anything.
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    /// Returns the country code of `ip`, or `None` when it is not a public address or the
    /// lookup fails. Results are cached; failures are logged and retried after a while.
    pub async fn country_code(&self, ip: IpAddr) -> Option<String> {
        let ipinfo = self.inner.as_ref()?;
        if !is_public(ip) {
            return None;
        }
        let lookup = ipinfo.cached(ip);
        lookup
            .country
            .get_or_init(|| async {
                match ipinfo.lookup(ip).await {
                    Ok(code) => {
                        tracing::debug!(%ip, code, "geolocated");
                        Some(code)
                    }
                    Err(e) => {
                        tracing::warn!(%ip, "geolocation failed: {e:#}");
                        None
                    }
                }
            })
            .await
            .clone()
    }
}

impl Ipinfo {
    /// Returns the cached lookup of `ip`, starting a new one when there is none or it expired.
    fn cached(&self, ip: IpAddr) -> Lookup {
        let now = Instant::now();
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(lookup) = cache.get(&ip)
            && lookup.is_fresh(now)
        {
            return lookup.clone();
        }
        if cache.len() >= PRUNE_ABOVE {
            cache.retain(|_, lookup| lookup.is_fresh(now));
        }
        let lookup = Lookup::new(now);
        cache.insert(ip, lookup.clone());
        lookup
    }

    async fn lookup(&self, ip: IpAddr) -> anyhow::Result<String> {
        let mut request = self.http.get(format!("{}/{ip}/country", self.base_url));
        if let Some(token) = &self.token {
            request = request.query(&[("token", token)]);
        }
        let body = request
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            // The request URL contains the ipinfo token; never include it in logged errors.
            .map_err(reqwest::Error::without_url)
            .context("request failed")?
            .text()
            .await
            .map_err(reqwest::Error::without_url)
            .context("failed to read response")?;

        let code = body.trim();
        // Reserved ranges come back as e.g. "bogon" rather than a country.
        if code.len() != 2 || !code.bytes().all(|b| b.is_ascii_uppercase()) {
            bail!("unexpected response {code:?}");
        }
        Ok(code.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, Query};
    use axum::http::StatusCode;
    use axum::routing::get;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const PRIVATE_TOKEN: &str = "private-ipinfo-token";

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn assert_token_redacted(error: &anyhow::Error) {
        // Match the full error chain rendered by the warning in `country_code`.
        let logged = format!("{error:#}");
        assert!(!logged.contains(PRIVATE_TOKEN), "{logged}");
        assert!(!logged.contains("token="), "{logged}");
    }

    /// Starts a fake ipinfo.io and returns its URL and request counter.
    async fn fake_ipinfo() -> (String, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let app = axum::Router::new().route(
            "/{ip}/country",
            get(
                move |Path(ip): Path<String>, Query(query): Query<HashMap<String, String>>| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    async move {
                        // Slow enough for concurrent lookups to overlap.
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        match (ip.as_str(), query.get("token").map(String::as_str)) {
                            ("8.8.8.8", _) => (StatusCode::OK, "US\n"),
                            ("2001:4860:4860::8888", _) => (StatusCode::OK, "SG"),
                            ("1.1.1.1", Some("secret")) => (StatusCode::OK, "AU"),
                            ("1.1.1.1", _) => (StatusCode::FORBIDDEN, "forbidden"),
                            _ => (StatusCode::OK, "bogon"),
                        }
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, hits)
    }

    #[tokio::test]
    async fn resolves_country_codes() {
        let (url, _) = fake_ipinfo().await;
        let geoip = GeoIp::with_base_url(&url, None).unwrap();
        assert_eq!(
            geoip.country_code(ip("8.8.8.8")).await.as_deref(),
            Some("US")
        );
        assert_eq!(
            geoip
                .country_code(ip("2001:4860:4860::8888"))
                .await
                .as_deref(),
            Some("SG")
        );
    }

    #[tokio::test]
    async fn authenticates_with_token() {
        let (url, _) = fake_ipinfo().await;
        let anonymous = GeoIp::with_base_url(&url, None).unwrap();
        assert_eq!(anonymous.country_code(ip("1.1.1.1")).await, None);

        let authenticated = GeoIp::with_base_url(&url, Some("secret".into())).unwrap();
        assert_eq!(
            authenticated.country_code(ip("1.1.1.1")).await.as_deref(),
            Some("AU")
        );
    }

    #[tokio::test]
    async fn http_errors_do_not_expose_token() {
        let (url, _) = fake_ipinfo().await;
        let geoip = GeoIp::with_base_url(&url, Some(PRIVATE_TOKEN.into())).unwrap();
        let error = geoip
            .inner
            .as_ref()
            .unwrap()
            .lookup(ip("1.1.1.1"))
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "request failed");
        assert_eq!(
            error.downcast_ref::<reqwest::Error>().unwrap().status(),
            Some(StatusCode::FORBIDDEN)
        );
        assert_token_redacted(&error);
    }

    #[tokio::test]
    async fn connection_errors_do_not_expose_token() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let geoip = GeoIp::with_base_url(&url, Some(PRIVATE_TOKEN.into())).unwrap();
        let error = geoip
            .inner
            .as_ref()
            .unwrap()
            .lookup(ip("8.8.8.8"))
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "request failed");
        assert!(error.downcast_ref::<reqwest::Error>().unwrap().is_connect());
        assert_token_redacted(&error);
    }

    #[tokio::test]
    async fn response_body_errors_do_not_expose_token() {
        use std::io::{BufRead, BufReader, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(TIMEOUT)).unwrap();
            let mut request = BufReader::new(&socket);
            loop {
                let mut line = String::new();
                assert!(request.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
            }
            // A successful response whose body ends before the declared length.
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nUS")
                .unwrap();
        });
        let geoip = GeoIp::with_base_url(&url, Some(PRIVATE_TOKEN.into())).unwrap();
        let error = geoip
            .inner
            .as_ref()
            .unwrap()
            .lookup(ip("8.8.8.8"))
            .await
            .unwrap_err();
        server.join().unwrap();

        assert_eq!(error.to_string(), "failed to read response");
        assert!(error.downcast_ref::<reqwest::Error>().is_some());
        assert_token_redacted(&error);
    }

    #[tokio::test]
    async fn rejects_non_country_responses() {
        let (url, _) = fake_ipinfo().await;
        let geoip = GeoIp::with_base_url(&url, None).unwrap();
        assert_eq!(geoip.country_code(ip("192.0.2.1")).await, None);
    }

    #[tokio::test]
    async fn skips_local_addresses_without_a_request() {
        let (url, hits) = fake_ipinfo().await;
        let geoip = GeoIp::with_base_url(&url, None).unwrap();
        assert_eq!(geoip.country_code(ip("10.0.0.1")).await, None);
        assert_eq!(geoip.country_code(ip("::1")).await, None);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn looks_up_each_address_once() {
        let (url, hits) = fake_ipinfo().await;
        let geoip = GeoIp::with_base_url(&url, None).unwrap();
        let (a, b) = tokio::join!(
            geoip.country_code(ip("8.8.8.8")),
            geoip.country_code(ip("8.8.8.8"))
        );
        assert_eq!((a.as_deref(), b.as_deref()), (Some("US"), Some("US")));
        assert_eq!(
            geoip.country_code(ip("8.8.8.8")).await.as_deref(),
            Some("US")
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        // Failures are cached as well, until they are retried.
        assert_eq!(geoip.country_code(ip("192.0.2.1")).await, None);
        assert_eq!(geoip.country_code(ip("192.0.2.1")).await, None);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn lookups_expire() {
        let now = Instant::now();
        let lookup = Lookup::new(now);
        assert!(lookup.is_fresh(now + FOUND_TTL), "in flight");

        lookup.country.set(None).unwrap();
        assert!(lookup.is_fresh(now));
        assert!(!lookup.is_fresh(now + RETRY_AFTER));

        let found = Lookup::new(now);
        found.country.set(Some("US".into())).unwrap();
        assert!(found.is_fresh(now + RETRY_AFTER));
        assert!(!found.is_fresh(now + FOUND_TTL));
    }

    #[tokio::test]
    async fn unreachable_service_yields_none() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let geoip = GeoIp::with_base_url(&url, None).unwrap();
        assert_eq!(geoip.country_code(ip("8.8.8.8")).await, None);
    }

    #[tokio::test]
    async fn disabled_resolver_finds_nothing() {
        assert_eq!(GeoIp::disabled().country_code(ip("8.8.8.8")).await, None);
    }
}

//! IP geolocation through ipinfo.io, falling back to GeoJS.

use aiom_protocol::is_public;
use anyhow::{Context, bail};
use reqwest::redirect::Policy;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

const IPINFO_URL: &str = "https://ipinfo.io";
const GEOJS_URL: &str = "https://get.geojs.io";
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
    inner: Option<Arc<Resolver>>,
}

#[derive(Debug)]
struct Resolver {
    http: reqwest::Client,
    /// Asked in order, until one of them finds the country.
    services: Vec<Service>,
    cache: Mutex<HashMap<IpAddr, Arc<Lookup>>>,
}

/// A geolocation service answering with the bare country code of an address.
#[derive(Debug)]
struct Service {
    api: Api,
    base_url: String,
}

#[derive(Debug)]
enum Api {
    /// ipinfo.io, or a compatible service.
    Ipinfo,
    GeoJs,
}

impl Service {
    fn new(api: Api, base_url: &str) -> Self {
        Self {
            api,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    fn url(&self, ip: IpAddr) -> String {
        let base_url = &self.base_url;
        match self.api {
            Api::Ipinfo => format!("{base_url}/{ip}/country"),
            Api::GeoJs => format!("{base_url}/v1/ip/country/{ip}"),
        }
    }
}

/// A lookup of one address, shared by everyone asking for it: concurrent requests for the
/// same address wait for one lookup.
#[derive(Debug)]
struct Lookup {
    started: Instant,
    /// The country an earlier lookup of the address found, kept when this one fails.
    previous: Option<String>,
    /// The country found, or `None` when every service failed.
    country: OnceCell<Option<String>>,
}

impl Lookup {
    fn new(now: Instant, previous: Option<String>) -> Self {
        Self {
            started: now,
            previous,
            country: OnceCell::new(),
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

    /// The country known for the address once this lookup completed: the one found, or else
    /// the one found before.
    fn known_country(&self) -> Option<&str> {
        self.country.get()?.as_deref().or(self.previous.as_deref())
    }
}

impl GeoIp {
    /// Looks addresses up on ipinfo.io, and on GeoJS when ipinfo.io fails, e.g. once its
    /// anonymous rate limit is reached. Neither needs a token.
    pub fn new() -> anyhow::Result<Self> {
        Self::with_fallback(IPINFO_URL, GEOJS_URL)
    }

    /// Like [`GeoIp::new`], against ipinfo.io at `ipinfo` and GeoJS at `geojs`.
    fn with_fallback(ipinfo: &str, geojs: &str) -> anyhow::Result<Self> {
        Self::with_services(vec![
            Service::new(Api::Ipinfo, ipinfo),
            Service::new(Api::GeoJs, geojs),
        ])
    }

    /// Looks addresses up on an ipinfo-compatible service at `base_url` only.
    pub fn with_base_url(base_url: &str) -> anyhow::Result<Self> {
        Self::with_services(vec![Service::new(Api::Ipinfo, base_url)])
    }

    fn with_services(services: Vec<Service>) -> anyhow::Result<Self> {
        crate::install_crypto_provider();
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(Policy::none())
            .user_agent(concat!("aiom-server/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            inner: Some(Arc::new(Resolver {
                http,
                services,
                cache: Mutex::default(),
            })),
        })
    }

    /// A resolver that never finds anything.
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    /// Returns the country code of `ip`, or `None` when it is not a public address or it was
    /// never found. Results are cached; failures are logged and retried after a while, and
    /// keep the country found for the address before.
    pub async fn country_code(&self, ip: IpAddr) -> Option<String> {
        self.country_code_at(ip, Instant::now()).await
    }

    async fn country_code_at(&self, ip: IpAddr, now: Instant) -> Option<String> {
        let resolver = self.inner.as_ref()?;
        if !is_public(ip) {
            return None;
        }
        let lookup = resolver.cached(ip, now);
        lookup.country.get_or_init(|| resolver.lookup(ip)).await;
        lookup.known_country().map(str::to_owned)
    }
}

impl Resolver {
    /// Returns the cached lookup of `ip`, starting a new one when there is none or it expired.
    fn cached(&self, ip: IpAddr, now: Instant) -> Arc<Lookup> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = match cache.get(&ip) {
            Some(lookup) if lookup.is_fresh(now) => return Arc::clone(lookup),
            Some(lookup) => lookup.known_country().map(str::to_owned),
            None => None,
        };
        if cache.len() >= PRUNE_ABOVE {
            cache.retain(|_, lookup| lookup.is_fresh(now));
        }
        let lookup = Arc::new(Lookup::new(now, previous));
        cache.insert(ip, Arc::clone(&lookup));
        lookup
    }

    /// Asks each service in turn for the country of `ip`.
    async fn lookup(&self, ip: IpAddr) -> Option<String> {
        for service in &self.services {
            match self.query(service, ip).await {
                Ok(code) => {
                    tracing::debug!(%ip, code, service = ?service.api, "geolocated");
                    return Some(code);
                }
                Err(e) => tracing::warn!(%ip, service = ?service.api, "geolocation failed: {e:#}"),
            }
        }
        None
    }

    async fn query(&self, service: &Service, ip: IpAddr) -> anyhow::Result<String> {
        let body = self
            .http
            .get(service.url(ip))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .context("request failed")?
            .text()
            .await
            .context("failed to read response")?;

        let code = body.trim();
        // Reserved ranges come back as e.g. "bogon" or nothing rather than a country.
        if code.len() != 2 || !code.bytes().all(|b| b.is_ascii_uppercase()) {
            bail!("unexpected response {code:?}");
        }
        Ok(code.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path;
    use axum::http::StatusCode;
    use axum::routing::get;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// Serves `app` on a free local port and returns its URL.
    async fn serve(app: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    /// A URL that nothing listens on.
    fn unreachable_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    }

    /// Starts a fake ipinfo.io and returns its URL and request counter.
    async fn fake_ipinfo() -> (String, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let app = axum::Router::new().route(
            "/{ip}/country",
            get(move |Path(ip): Path<String>| {
                counter.fetch_add(1, Ordering::SeqCst);
                async move {
                    // Slow enough for concurrent lookups to overlap.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    match ip.as_str() {
                        "8.8.8.8" => (StatusCode::OK, "US\n"),
                        "2001:4860:4860::8888" => (StatusCode::OK, "SG"),
                        "1.1.1.1" => (StatusCode::TOO_MANY_REQUESTS, "rate limited"),
                        _ => (StatusCode::OK, "bogon"),
                    }
                }
            }),
        );
        (serve(app).await, hits)
    }

    /// Starts a fake GeoJS and returns its URL and request counter.
    async fn fake_geojs() -> (String, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let app = axum::Router::new().route(
            "/v1/ip/country/{ip}",
            get(move |Path(ip): Path<String>| {
                counter.fetch_add(1, Ordering::SeqCst);
                async move {
                    match ip.as_str() {
                        "1.1.1.1" => "AU\n",
                        // GeoJS answers addresses it cannot place with an empty body.
                        _ => "\n",
                    }
                }
            }),
        );
        (serve(app).await, hits)
    }

    /// A resolver asking the fake ipinfo.io first, then the fake GeoJS.
    async fn fakes_with_fallback() -> (GeoIp, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let (ipinfo, ipinfo_hits) = fake_ipinfo().await;
        let (geojs, geojs_hits) = fake_geojs().await;
        let geoip = GeoIp::with_fallback(&ipinfo, &geojs).unwrap();
        (geoip, ipinfo_hits, geojs_hits)
    }

    #[tokio::test]
    async fn resolves_country_codes() {
        let (url, _) = fake_ipinfo().await;
        let geoip = GeoIp::with_base_url(&url).unwrap();
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
    async fn falls_back_when_ipinfo_fails() {
        let (geoip, ipinfo_hits, geojs_hits) = fakes_with_fallback().await;

        // ipinfo.io answers, so GeoJS is not asked.
        assert_eq!(
            geoip.country_code(ip("8.8.8.8")).await.as_deref(),
            Some("US")
        );
        assert_eq!(geojs_hits.load(Ordering::SeqCst), 0);

        // ipinfo.io rate limits this address, so GeoJS answers.
        assert_eq!(
            geoip.country_code(ip("1.1.1.1")).await.as_deref(),
            Some("AU")
        );
        assert_eq!(ipinfo_hits.load(Ordering::SeqCst), 2);
        assert_eq!(geojs_hits.load(Ordering::SeqCst), 1);

        // The answer from the fallback is cached like any other.
        assert_eq!(
            geoip.country_code(ip("1.1.1.1")).await.as_deref(),
            Some("AU")
        );
        assert_eq!(ipinfo_hits.load(Ordering::SeqCst), 2);
        assert_eq!(geojs_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn falls_back_when_ipinfo_is_unreachable() {
        let (geojs, _) = fake_geojs().await;
        let geoip = GeoIp::with_fallback(&unreachable_url(), &geojs).unwrap();
        assert_eq!(
            geoip.country_code(ip("1.1.1.1")).await.as_deref(),
            Some("AU")
        );
    }

    #[tokio::test]
    async fn yields_none_when_every_service_fails() {
        let (geoip, ipinfo_hits, geojs_hits) = fakes_with_fallback().await;
        assert_eq!(geoip.country_code(ip("192.0.2.1")).await, None);
        assert_eq!(geoip.country_code(ip("192.0.2.1")).await, None);
        // Each service was asked once; the failure is cached until it is retried.
        assert_eq!(ipinfo_hits.load(Ordering::SeqCst), 1);
        assert_eq!(geojs_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rejects_non_country_responses() {
        let (url, _) = fake_ipinfo().await;
        let geoip = GeoIp::with_base_url(&url).unwrap();
        assert_eq!(geoip.country_code(ip("192.0.2.1")).await, None);
    }

    #[tokio::test]
    async fn skips_local_addresses_without_a_request() {
        let (url, hits) = fake_ipinfo().await;
        let geoip = GeoIp::with_base_url(&url).unwrap();
        assert_eq!(geoip.country_code(ip("10.0.0.1")).await, None);
        assert_eq!(geoip.country_code(ip("::1")).await, None);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn looks_up_each_address_once() {
        let (url, hits) = fake_ipinfo().await;
        let geoip = GeoIp::with_base_url(&url).unwrap();
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
        let lookup = Lookup::new(now, None);
        assert!(lookup.is_fresh(now + FOUND_TTL), "in flight");

        lookup.country.set(None).unwrap();
        assert!(lookup.is_fresh(now));
        assert!(!lookup.is_fresh(now + RETRY_AFTER));

        // A failure keeping an earlier country is retried just as soon.
        let kept = Lookup::new(now, Some("US".into()));
        kept.country.set(None).unwrap();
        assert_eq!(kept.known_country(), Some("US"));
        assert!(!kept.is_fresh(now + RETRY_AFTER));

        let found = Lookup::new(now, None);
        found.country.set(Some("US".into())).unwrap();
        assert!(found.is_fresh(now + RETRY_AFTER));
        assert!(!found.is_fresh(now + FOUND_TTL));
    }

    #[tokio::test]
    async fn failed_lookups_keep_the_country_found_before() {
        let down = Arc::new(AtomicBool::new(false));
        let hits = Arc::new(AtomicUsize::new(0));
        let (switch, counter) = (down.clone(), hits.clone());
        let url = serve(axum::Router::new().route(
            "/{ip}/country",
            get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                let answer = if switch.load(Ordering::SeqCst) {
                    (StatusCode::SERVICE_UNAVAILABLE, "down")
                } else {
                    (StatusCode::OK, "US")
                };
                async move { answer }
            }),
        ))
        .await;
        let geoip = GeoIp::with_base_url(&url).unwrap();
        let (address, now) = (ip("8.8.8.8"), Instant::now());
        let at = |later| geoip.country_code_at(address, now + later);

        assert_eq!(at(Duration::ZERO).await.as_deref(), Some("US"));

        // The country expired and the service is down: the known one is kept.
        down.store(true, Ordering::SeqCst);
        assert_eq!(at(FOUND_TTL).await.as_deref(), Some("US"));
        assert_eq!(hits.load(Ordering::SeqCst), 2);

        // It is retried as after any failure, and kept for as long as the service is down.
        assert_eq!(at(FOUND_TTL).await.as_deref(), Some("US"));
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(at(FOUND_TTL + RETRY_AFTER).await.as_deref(), Some("US"));
        assert_eq!(hits.load(Ordering::SeqCst), 3);

        // Back up, the service answers again, and its answer is cached for long.
        down.store(false, Ordering::SeqCst);
        assert_eq!(at(FOUND_TTL + 2 * RETRY_AFTER).await.as_deref(), Some("US"));
        assert_eq!(hits.load(Ordering::SeqCst), 4);
        assert_eq!(at(FOUND_TTL + 3 * RETRY_AFTER).await.as_deref(), Some("US"));
        assert_eq!(hits.load(Ordering::SeqCst), 4);

        // Another address never found has nothing to keep.
        down.store(true, Ordering::SeqCst);
        assert_eq!(geoip.country_code_at(ip("1.1.1.1"), now).await, None);
    }

    #[tokio::test]
    async fn unreachable_service_yields_none() {
        let geoip = GeoIp::with_base_url(&unreachable_url()).unwrap();
        assert_eq!(geoip.country_code(ip("8.8.8.8")).await, None);
    }

    #[tokio::test]
    async fn disabled_resolver_finds_nothing() {
        assert_eq!(GeoIp::disabled().country_code(ip("8.8.8.8")).await, None);
    }
}

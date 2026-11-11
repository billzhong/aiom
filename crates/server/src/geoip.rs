//! IP geolocation through ipinfo.io, falling back to GeoJS.

use aiom_protocol::as_public;
use anyhow::{Context, bail};
use reqwest::StatusCode;
use reqwest::redirect::Policy;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

use crate::lock;

const IPINFO_URL: &str = "https://ipinfo.io";
const GEOJS_URL: &str = "https://get.geojs.io";
const TIMEOUT: Duration = Duration::from_secs(3);
/// How long what the services answered for an address is reused, a country or none.
const FOUND_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// How long to wait before looking up an address again after a failure, and before asking a
/// service that rate limited us again.
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
    cache: Mutex<Cache>,
}

#[derive(Debug, Default)]
struct Cache {
    lookups: HashMap<IpAddr, Arc<Lookup>>,
    /// The size at which expired lookups are dropped next, once above [`PRUNE_ABOVE`].
    prune_at: usize,
}

/// A geolocation service answering with the bare country code of an address.
#[derive(Debug)]
struct Service {
    api: Api,
    base_url: String,
    /// Until when the service is not asked, after it rate limited us.
    paused_until: Mutex<Option<Instant>>,
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
            paused_until: Mutex::default(),
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
    /// The country an earlier lookup of the address found, kept when this one finds none.
    previous: Option<String>,
    country: OnceCell<Outcome>,
}

/// What looking an address up came to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Found(String),
    /// Every service answered, without a country, as for reserved addresses.
    Unknown,
    /// No service found the country, and some could not be asked.
    Failed,
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
            Some(Outcome::Found(_) | Outcome::Unknown) => FOUND_TTL,
            Some(Outcome::Failed) => RETRY_AFTER,
        };
        now < self.started + ttl
    }

    /// The country known for the address once this lookup completed: the one found, or else
    /// the one found before.
    fn known_country(&self) -> Option<&str> {
        match self.country.get()? {
            Outcome::Found(country) => Some(country),
            Outcome::Unknown | Outcome::Failed => self.previous.as_deref(),
        }
    }
}

impl GeoIp {
    /// Looks addresses up on ipinfo.io, and on GeoJS when ipinfo.io does not find the country.
    /// Once ipinfo.io rate limits, as when its anonymous limit is reached, only GeoJS is asked
    /// for a while. Neither needs a token.
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
    /// never found. Results are cached, answers without a country as long as countries;
    /// failures are logged and retried after a while. Either keeps the country found for the
    /// address before.
    pub async fn country_code(&self, ip: IpAddr) -> Option<String> {
        self.country_code_at(ip, Instant::now()).await
    }

    async fn country_code_at(&self, ip: IpAddr, now: Instant) -> Option<String> {
        let resolver = self.inner.as_ref()?;
        let ip = as_public(ip)?;
        let lookup = resolver.cached(ip, now);
        lookup
            .country
            .get_or_init(|| resolver.lookup(ip, now))
            .await;
        lookup.known_country().map(str::to_owned)
    }

    /// What [`country_code`](Self::country_code) returns for `ip` when it would not have to
    /// wait for a lookup, else `None`. Starts no lookup itself.
    pub(crate) fn known_country_code(&self, ip: IpAddr) -> Option<Option<String>> {
        self.known_country_code_at(ip, Instant::now())
    }

    fn known_country_code_at(&self, ip: IpAddr, now: Instant) -> Option<Option<String>> {
        let (Some(resolver), Some(ip)) = (self.inner.as_ref(), as_public(ip)) else {
            return Some(None);
        };
        let cache = lock(&resolver.cache);
        let lookup = cache
            .lookups
            .get(&ip)
            .filter(|lookup| lookup.country.initialized() && lookup.is_fresh(now))?;
        Some(lookup.known_country().map(str::to_owned))
    }
}

impl Resolver {
    /// Returns the cached lookup of `ip`, starting a new one when there is none or it expired.
    fn cached(&self, ip: IpAddr, now: Instant) -> Arc<Lookup> {
        let mut cache = lock(&self.cache);
        let previous = match cache.lookups.get(&ip) {
            Some(lookup) if lookup.is_fresh(now) => return Arc::clone(lookup),
            Some(lookup) => lookup.known_country().map(str::to_owned),
            None => None,
        };
        if cache.lookups.len() >= cache.prune_at.max(PRUNE_ABOVE) {
            cache.lookups.retain(|_, lookup| lookup.is_fresh(now));
            // Pruning again only once the cache doubled keeps the cost of inserting constant on
            // average, however many lookups are fresh.
            cache.prune_at = 2 * cache.lookups.len();
        }
        let lookup = Arc::new(Lookup::new(now, previous));
        cache.lookups.insert(ip, Arc::clone(&lookup));
        lookup
    }

    /// Asks each service in turn for the country of `ip`.
    async fn lookup(&self, ip: IpAddr, now: Instant) -> Outcome {
        let mut errors = Vec::new();
        for service in &self.services {
            match self.query(service, ip, now).await {
                Ok(Some(code)) => {
                    tracing::debug!(%ip, code, service = ?service.api, "geolocated");
                    return Outcome::Found(code);
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::debug!(%ip, service = ?service.api, "geolocation failed: {e:#}");
                    errors.push(format!("{:?}: {e:#}", service.api));
                }
            }
        }
        if errors.is_empty() {
            Outcome::Unknown
        } else {
            tracing::warn!(%ip, "geolocation failed: {}", errors.join("; "));
            Outcome::Failed
        }
    }

    /// Asks `service` for the country of `ip`; `None` when it answers without one.
    async fn query(
        &self,
        service: &Service,
        ip: IpAddr,
        now: Instant,
    ) -> anyhow::Result<Option<String>> {
        if lock(&service.paused_until).is_some_and(|until| now < until) {
            bail!("paused after rate limiting");
        }
        let response = self
            .http
            .get(service.url(ip))
            .send()
            .await
            .context("request failed")?;
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            *lock(&service.paused_until) = Some(now + RETRY_AFTER);
            bail!("rate limited, pausing for {} s", RETRY_AFTER.as_secs());
        }
        let body = response
            .error_for_status()
            .context("request failed")?
            .text()
            .await
            .context("failed to read response")?;

        let code = body.trim();
        if code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase()) {
            return Ok(Some(code.to_owned()));
        }
        // Reserved ranges come back as e.g. "bogon" or nothing rather than a country.
        tracing::debug!(%ip, service = ?service.api, "no country: {code:?}");
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path;
    use axum::http::StatusCode;
    use axum::routing::get;
    use std::net::Ipv4Addr;
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

    /// A URL whose server closes every connection right away. It keeps its port, which
    /// another test could get otherwise.
    async fn unreachable_url() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { while listener.accept().await.is_ok() {} });
        url
    }

    /// Starts a service that is down and returns its URL and request counter.
    async fn fake_down() -> (String, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let app = axum::Router::new().fallback(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { StatusCode::SERVICE_UNAVAILABLE }
        });
        (serve(app).await, hits)
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

    /// Reads the request counters of a first service and its fallback.
    fn hits_of(first: Arc<AtomicUsize>, fallback: Arc<AtomicUsize>) -> impl Fn() -> (usize, usize) {
        move || {
            (
                first.load(Ordering::SeqCst),
                fallback.load(Ordering::SeqCst),
            )
        }
    }

    /// A resolver asking the fake ipinfo.io first, then the fake GeoJS, and how often each
    /// was asked.
    async fn fakes_with_fallback() -> (GeoIp, impl Fn() -> (usize, usize)) {
        let (ipinfo, ipinfo_hits) = fake_ipinfo().await;
        let (geojs, geojs_hits) = fake_geojs().await;
        let geoip = GeoIp::with_fallback(&ipinfo, &geojs).unwrap();
        (geoip, hits_of(ipinfo_hits, geojs_hits))
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
        let (geoip, hits) = fakes_with_fallback().await;

        // ipinfo.io answers, so GeoJS is not asked.
        assert_eq!(
            geoip.country_code(ip("8.8.8.8")).await.as_deref(),
            Some("US")
        );
        assert_eq!(hits(), (1, 0));

        // ipinfo.io rate limits this address, so GeoJS answers.
        assert_eq!(
            geoip.country_code(ip("1.1.1.1")).await.as_deref(),
            Some("AU")
        );
        assert_eq!(hits(), (2, 1));

        // The answer from the fallback is cached like any other.
        assert_eq!(
            geoip.country_code(ip("1.1.1.1")).await.as_deref(),
            Some("AU")
        );
        assert_eq!(hits(), (2, 1));
    }

    #[tokio::test]
    async fn falls_back_when_ipinfo_is_unreachable() {
        let (geojs, _) = fake_geojs().await;
        let geoip = GeoIp::with_fallback(&unreachable_url().await, &geojs).unwrap();
        assert_eq!(
            geoip.country_code(ip("1.1.1.1")).await.as_deref(),
            Some("AU")
        );
    }

    #[tokio::test]
    async fn yields_none_when_every_service_fails() {
        let (down, down_hits) = fake_down().await;
        let (geojs, geojs_hits) = fake_geojs().await;
        let geoip = GeoIp::with_fallback(&down, &geojs).unwrap();
        let hits = hits_of(down_hits, geojs_hits);
        let (address, now) = (ip("8.8.8.8"), Instant::now());
        assert_eq!(geoip.country_code_at(address, now).await, None);
        assert_eq!(geoip.country_code_at(address, now).await, None);
        // Each service was asked once; the failure is cached until it is retried.
        assert_eq!(hits(), (1, 1));

        // A service that is down is asked again with the next lookup.
        assert_eq!(
            geoip.country_code_at(address, now + RETRY_AFTER).await,
            None
        );
        assert_eq!(hits(), (2, 2));
    }

    #[tokio::test]
    async fn keeps_answers_without_a_country_for_long() {
        let (geoip, hits) = fakes_with_fallback().await;
        let (address, now) = (ip("192.0.2.1"), Instant::now());
        // Neither service places this address, which is no failure to retry soon.
        assert_eq!(geoip.country_code_at(address, now).await, None);
        assert_eq!(
            geoip.country_code_at(address, now + RETRY_AFTER).await,
            None
        );
        assert_eq!(hits(), (1, 1));

        assert_eq!(geoip.country_code_at(address, now + FOUND_TTL).await, None);
        assert_eq!(hits(), (2, 2));
    }

    #[tokio::test]
    async fn pauses_a_service_that_rate_limits() {
        let (geoip, hits) = fakes_with_fallback().await;
        let now = Instant::now();
        // ipinfo.io rate limits this address, so GeoJS answers.
        let rate_limited = geoip.country_code_at(ip("1.1.1.1"), now).await;
        assert_eq!(rate_limited.as_deref(), Some("AU"));
        assert_eq!(hits(), (1, 1));

        // For a while, only GeoJS is asked, which cannot place this one. Not having asked
        // ipinfo.io, the lookup failed, and is retried soon.
        let address = ip("8.8.8.8");
        assert_eq!(geoip.country_code_at(address, now).await, None);
        assert_eq!(hits(), (1, 2));

        let later = now + RETRY_AFTER;
        let retried = geoip.country_code_at(address, later).await;
        assert_eq!(retried.as_deref(), Some("US"));
        assert_eq!(hits(), (2, 2));
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

        // Addresses the service does not place are cached as well.
        assert_eq!(geoip.country_code(ip("192.0.2.1")).await, None);
        assert_eq!(geoip.country_code(ip("192.0.2.1")).await, None);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn lookups_expire() {
        let now = Instant::now();
        let lookup = Lookup::new(now, None);
        assert!(lookup.is_fresh(now + FOUND_TTL), "in flight");

        lookup.country.set(Outcome::Failed).unwrap();
        assert!(lookup.is_fresh(now));
        assert!(!lookup.is_fresh(now + RETRY_AFTER));

        // A failure keeping an earlier country is retried just as soon.
        let kept = Lookup::new(now, Some("US".into()));
        kept.country.set(Outcome::Failed).unwrap();
        assert_eq!(kept.known_country(), Some("US"));
        assert!(!kept.is_fresh(now + RETRY_AFTER));

        let found = Lookup::new(now, None);
        found.country.set(Outcome::Found("US".into())).unwrap();
        assert!(found.is_fresh(now + RETRY_AFTER));
        assert!(!found.is_fresh(now + FOUND_TTL));

        // An answer without a country lasts as long, and keeps an earlier country too.
        let unknown = Lookup::new(now, Some("US".into()));
        unknown.country.set(Outcome::Unknown).unwrap();
        assert_eq!(unknown.known_country(), Some("US"));
        assert!(unknown.is_fresh(now + RETRY_AFTER));
        assert!(!unknown.is_fresh(now + FOUND_TTL));
    }

    #[test]
    fn prunes_expired_lookups_once_the_cache_doubled() {
        let geoip = GeoIp::with_base_url("http://unused.invalid").unwrap();
        let resolver = geoip.inner.as_ref().unwrap();
        let now = Instant::now();
        let mut next = 0x0808_0000_u32;
        let mut add = |at: Instant, outcome: Outcome| {
            next += 1;
            let lookup = resolver.cached(IpAddr::V4(Ipv4Addr::from(next)), at);
            lookup.country.set(outcome).unwrap();
        };
        let size = || lock(&resolver.cache).lookups.len();

        for _ in 0..PRUNE_ABOVE {
            add(now, Outcome::Failed);
        }
        // All still fresh, so none is dropped, and the cache may double before it is pruned
        // again.
        add(now, Outcome::Found("US".into()));
        assert_eq!(size(), PRUNE_ABOVE + 1);
        let later = now + RETRY_AFTER;
        while size() < 2 * PRUNE_ABOVE {
            add(later, Outcome::Found("US".into()));
        }
        assert_eq!(size(), 2 * PRUNE_ABOVE);

        // Doubled, the failures that expired are dropped.
        add(later, Outcome::Found("US".into()));
        assert_eq!(size(), PRUNE_ABOVE + 1);
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
    async fn looks_up_mapped_ipv4_addresses_as_ipv4() {
        let (url, hits) = fake_ipinfo().await;
        let geoip = GeoIp::with_base_url(&url).unwrap();
        assert_eq!(
            geoip.country_code(ip("::ffff:8.8.8.8")).await.as_deref(),
            Some("US")
        );
        assert_eq!(
            geoip.country_code(ip("8.8.8.8")).await.as_deref(),
            Some("US")
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn knows_only_completed_lookups() {
        let (url, hits) = fake_ipinfo().await;
        let geoip = GeoIp::with_base_url(&url).unwrap();
        let (address, now) = (ip("8.8.8.8"), Instant::now());
        assert_eq!(geoip.known_country_code_at(address, now), None);
        assert_eq!(hits.load(Ordering::SeqCst), 0);

        // In flight, as the fake takes 50 ms to answer, it is not known yet.
        let lookup = tokio::spawn({
            let geoip = geoip.clone();
            async move { geoip.country_code_at(address, now).await }
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(geoip.known_country_code_at(address, now), None);
        assert_eq!(lookup.await.unwrap().as_deref(), Some("US"));

        let known = geoip.known_country_code_at(address, now);
        assert_eq!(known, Some(Some("US".into())));
        let mapped = geoip.known_country_code_at(ip("::ffff:8.8.8.8"), now);
        assert_eq!(mapped, Some(Some("US".into())));
        // Once it expires, it has to be looked up again.
        assert_eq!(geoip.known_country_code_at(address, now + FOUND_TTL), None);

        // Addresses the service does not place are known as well.
        assert_eq!(geoip.country_code_at(ip("192.0.2.1"), now).await, None);
        assert_eq!(
            geoip.known_country_code_at(ip("192.0.2.1"), now),
            Some(None)
        );

        // Addresses that are never looked up are known to have no country.
        assert_eq!(geoip.known_country_code_at(ip("10.0.0.1"), now), Some(None));
        let disabled = GeoIp::disabled().known_country_code_at(address, now);
        assert_eq!(disabled, Some(None));
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn unreachable_service_yields_none() {
        let geoip = GeoIp::with_base_url(&unreachable_url().await).unwrap();
        assert_eq!(geoip.country_code(ip("8.8.8.8")).await, None);
    }

    #[tokio::test]
    async fn disabled_resolver_finds_nothing() {
        assert_eq!(GeoIp::disabled().country_code(ip("8.8.8.8")).await, None);
    }
}

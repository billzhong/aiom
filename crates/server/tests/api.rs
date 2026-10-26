//! Drives a running server over real HTTP and WebSocket connections.

#![allow(clippy::unwrap_used)]

use aiom_protocol::{
    AgentData, AgentReport, DASHBOARD_PING, Host, NodeSnapshot, PROTOCOL_VERSION, PublicIp, Stats,
    decode, encode, unix_now,
};
use aiom_server::{App, Config, GeoIp, HEALTH_PATH};
use axum::extract::Path;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderName;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const TOKEN: &str = "test-token";
const TIMEOUT: Duration = Duration::from_secs(10);

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct TestServer {
    addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<std::io::Result<()>>,
}

fn config(args: &[&str]) -> Config {
    Config::try_parse_from(
        ["aiom-server", "--token", TOKEN]
            .into_iter()
            .chain(args.iter().copied()),
    )
    .unwrap()
}

fn app(args: &[&str], geoip: GeoIp) -> App {
    App::new(&config(args), geoip)
        .unwrap()
        .with_snapshot_ttl(Duration::ZERO)
}

impl TestServer {
    async fn start(args: &[&str], geoip: GeoIp) -> Self {
        Self::serve(app(args, geoip)).await
    }

    async fn serve(app: App) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown, signal) = oneshot::channel();
        let task = tokio::spawn(app.serve(listener, async {
            signal.await.ok();
        }));
        Self {
            addr,
            shutdown,
            task,
        }
    }

    async fn default() -> Self {
        Self::start(&[], GeoIp::disabled()).await
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    async fn agent(&self) -> Socket {
        self.agent_with(&[]).await
    }

    async fn agent_with(&self, headers: &[(&str, &str)]) -> Socket {
        self.agent_at("/agent", Some(TOKEN), headers).await.unwrap()
    }

    async fn agent_at(
        &self,
        path: &str,
        token: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Result<Socket, tungstenite::Error> {
        let mut request = format!("ws://{}{path}", self.addr)
            .into_client_request()
            .unwrap();
        if let Some(token) = token {
            request
                .headers_mut()
                .insert("Authorization", format!("Bearer {token}").parse().unwrap());
        }
        for &(name, value) in headers {
            request
                .headers_mut()
                .insert(HeaderName::try_from(name).unwrap(), value.parse().unwrap());
        }
        connect_async(request).await.map(|(socket, _)| socket)
    }

    /// Connects a new agent reporting as `name`, retrying while the server still holds the
    /// name for a connection that is going away.
    async fn rejoin(&self, name: &str) -> Socket {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let mut agent = self.agent().await;
            send_report(&mut agent, &report(name, "")).await;
            if is_accepted(&mut agent).await {
                return agent;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{name} was never accepted"
            );
        }
    }

    async fn dashboard(&self) -> Socket {
        self.dashboard_at("/ws").await
    }

    async fn dashboard_at(&self, path: &str) -> Socket {
        connect_async(format!("ws://{}{path}", self.addr))
            .await
            .unwrap()
            .0
    }

    async fn stop(self) {
        self.shutdown.send(()).unwrap();
        tokio::time::timeout(TIMEOUT, self.task)
            .await
            .expect("server did not shut down")
            .unwrap()
            .unwrap();
    }
}

fn report(name: &str, ipv4: &str) -> AgentReport {
    AgentReport {
        version: PROTOCOL_VERSION,
        timestamp: 0,
        data: AgentData {
            name: name.into(),
            host: Host {
                agent: "test".into(),
                os: "Debian GNU/Linux".into(),
                total_memory: 4096,
                ..Default::default()
            },
            stats: Stats {
                cpu_usage: 12.5,
                used_memory: 1024,
                ..Default::default()
            },
            ip: PublicIp {
                public_ipv4: ipv4.into(),
                public_ipv6: String::new(),
            },
        },
    }
}

async fn send_report(agent: &mut Socket, report: &AgentReport) {
    agent
        .send(Message::binary(encode(report).unwrap()))
        .await
        .unwrap();
}

async fn poll(dashboard: &mut Socket) -> Vec<NodeSnapshot> {
    dashboard.send(Message::text(DASHBOARD_PING)).await.unwrap();
    loop {
        let message = tokio::time::timeout(TIMEOUT, dashboard.next())
            .await
            .expect("no reply to ping")
            .expect("dashboard socket closed")
            .unwrap();
        if let Message::Binary(payload) = message {
            return decode(&payload, 1 << 20).unwrap();
        }
    }
}

/// Polls until `done` holds for the node list, which the server stores asynchronously.
async fn poll_until(
    dashboard: &mut Socket,
    done: impl Fn(&[NodeSnapshot]) -> bool,
) -> Vec<NodeSnapshot> {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let nodes = poll(dashboard).await;
        if done(&nodes) {
            return nodes;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out, last node list: {nodes:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Whether the server keeps `agent` connected after its first report: rejected agents are
/// closed right away.
async fn is_accepted(agent: &mut Socket) -> bool {
    let window = tokio::time::sleep(Duration::from_millis(200));
    tokio::pin!(window);
    loop {
        tokio::select! {
            () = &mut window => return true,
            message = agent.next() => match message {
                Some(Ok(Message::Close(_)) | Err(_)) | None => return false,
                Some(Ok(_)) => continue,
            },
        }
    }
}

async fn expect_close(socket: &mut Socket) -> Option<CloseCode> {
    loop {
        match tokio::time::timeout(TIMEOUT, socket.next())
            .await
            .expect("socket was not closed")
        {
            Some(Ok(Message::Close(frame))) => return frame.map(|f| f.code),
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => return None,
        }
    }
}

#[tokio::test]
async fn health_check_and_unknown_routes() {
    let server = TestServer::default().await;
    let health = reqwest_client()
        .get(server.url(HEALTH_PATH))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    assert_eq!(health.text().await.unwrap(), "ok");

    let missing = reqwest_client()
        .get(server.url("/nope"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
    server.stop().await;
}

#[tokio::test]
async fn agent_route_requires_token() {
    let server = TestServer::default().await;
    for token in [None, Some("wrong-token")] {
        match server.agent_at("/agent", token, &[]).await {
            Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 401),
            other => panic!("expected 401 for {token:?}, got {other:?}"),
        }
    }
    server.stop().await;
}

#[tokio::test]
async fn dashboard_sees_agent_reports() {
    let server = TestServer::default().await;
    let mut dashboard = server.dashboard().await;
    assert!(poll(&mut dashboard).await.is_empty());

    let mut agent = server.agent().await;
    send_report(&mut agent, &report("node-1", "")).await;
    let nodes = poll_until(&mut dashboard, |nodes| !nodes.is_empty()).await;

    let node = &nodes[0];
    assert_eq!(node.name, "node-1");
    assert_eq!(node.host.os, "Debian GNU/Linux");
    assert_eq!(node.stats.cpu_usage, 12.5);
    assert_eq!(node.geoip, "");
    // The server stamps reports with its own clock, so skewed client clocks do not matter.
    assert!(
        (unix_now() - node.timestamp).abs() <= 5,
        "{}",
        node.timestamp
    );
    server.stop().await;
}

#[tokio::test]
async fn keeps_only_latest_report_per_node_in_natural_order() {
    let server = TestServer::default().await;
    let mut agent = server.agent().await;
    for name in ["node-10", "node-2", "node-1"] {
        send_report(&mut agent, &report(name, "")).await;
    }
    let mut latest = report("node-2", "");
    latest.data.stats.cpu_usage = 99.0;
    send_report(&mut agent, &latest).await;

    let mut dashboard = server.dashboard().await;
    let nodes = poll_until(&mut dashboard, |nodes| {
        nodes.iter().any(|n| n.stats.cpu_usage == 99.0)
    })
    .await;
    let names: Vec<_> = nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["node-1", "node-2", "node-10"]);
    server.stop().await;
}

#[tokio::test]
async fn accepts_reports_with_missing_and_unknown_fields() {
    let server = TestServer::default().await;
    let mut agent = server.agent().await;
    // As a client of another version might send it: fields missing, plus an unknown one.
    let payload = serde_json::json!({
        "version": 1,
        "timestamp": 1,
        "data": {
            "name": "other-version",
            "host": {"agent": "0.0.1", "os": "Ubuntu", "cpu_count": 2, "v12n": "kvm"},
            "stats": {"cpu_usage": 3.0, "tcp_count": 7, "extra": true},
            "ip": {"public_ipv4": "", "public_ipv6": ""}
        }
    });
    agent
        .send(Message::binary(encode(&payload).unwrap()))
        .await
        .unwrap();

    let mut dashboard = server.dashboard().await;
    let nodes = poll_until(&mut dashboard, |nodes| !nodes.is_empty()).await;
    assert_eq!(nodes[0].host.v12n, "kvm");
    assert_eq!(nodes[0].stats.tcp_count, 7);
    server.stop().await;
}

#[tokio::test]
async fn closes_connection_on_unsupported_version() {
    let server = TestServer::default().await;
    let mut agent = server.agent().await;
    let mut future = report("node", "");
    future.version = PROTOCOL_VERSION + 1;
    send_report(&mut agent, &future).await;
    assert_eq!(expect_close(&mut agent).await, Some(CloseCode::Protocol));

    let mut dashboard = server.dashboard().await;
    assert!(poll(&mut dashboard).await.is_empty());
    server.stop().await;
}

#[tokio::test]
async fn closes_connection_on_malformed_report() {
    let server = TestServer::default().await;
    let mut agent = server.agent().await;
    agent
        .send(Message::binary(b"not gzip".to_vec()))
        .await
        .unwrap();
    assert_eq!(expect_close(&mut agent).await, Some(CloseCode::Invalid));
    server.stop().await;
}

#[tokio::test]
async fn accepts_dashboards_served_from_other_origins() {
    let server = TestServer::default().await;
    // Browsers send the origin of the page, but WebSockets are not subject to CORS.
    let mut request = format!("ws://{}/ws", server.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Origin", "https://dashboard.example".parse().unwrap());
    let (mut dashboard, _) = connect_async(request).await.unwrap();
    assert!(poll(&mut dashboard).await.is_empty());
    server.stop().await;
}

#[tokio::test]
async fn dashboard_ignores_other_messages() {
    let server = TestServer::default().await;
    let mut dashboard = server.dashboard().await;
    dashboard.send(Message::text("hello")).await.unwrap();
    dashboard
        .send(Message::binary(b"ping".to_vec()))
        .await
        .unwrap();
    // Only the real ping is answered.
    assert!(poll(&mut dashboard).await.is_empty());
    let extra = tokio::time::timeout(Duration::from_millis(200), dashboard.next()).await;
    assert!(extra.is_err(), "unexpected message: {extra:?}");
    server.stop().await;
}

#[tokio::test]
async fn serves_custom_routes() {
    // Matched literally, not as a capture and a wildcard.
    let server = TestServer::start(
        &["--dashboard-path", "/:live", "--agent-path", "/*report"],
        GeoIp::disabled(),
    )
    .await;
    let mut agent = server.agent_at("/*report", Some(TOKEN), &[]).await.unwrap();
    send_report(&mut agent, &report("custom", "")).await;
    let mut dashboard = server.dashboard_at("/:live").await;
    poll_until(&mut dashboard, |nodes| !nodes.is_empty()).await;

    for path in ["/ws", "/live", "/report"] {
        let response = reqwest_client().get(server.url(path)).send().await.unwrap();
        assert_eq!(response.status(), 404, "{path}");
    }
    server.stop().await;
}

#[tokio::test]
async fn rejects_a_second_client_reporting_as_a_connected_node() {
    let server = TestServer::default().await;
    let mut first = server.agent().await;
    send_report(&mut first, &report("twin", "")).await;
    let mut dashboard = server.dashboard().await;
    poll_until(&mut dashboard, |nodes| !nodes.is_empty()).await;

    let mut second = server.agent().await;
    send_report(&mut second, &report("twin", "")).await;
    assert_eq!(expect_close(&mut second).await, Some(CloseCode::Policy));

    // The first client carries on undisturbed.
    let mut update = report("twin", "");
    update.data.stats.cpu_usage = 33.0;
    send_report(&mut first, &update).await;
    let nodes = poll_until(&mut dashboard, |nodes| nodes[0].stats.cpu_usage == 33.0).await;
    assert_eq!(nodes.len(), 1);

    // Once it is gone, the name is free again.
    first.close(None).await.unwrap();
    server.rejoin("twin").await;
    server.stop().await;
}

/// Keepalive settings short enough for tests.
fn quick_keepalive(app: App) -> App {
    app.with_keepalive(Duration::from_millis(50), Duration::from_millis(300))
}

/// Sends a report as `name` every 50 ms, until the connection fails.
fn keep_reporting(mut agent: Socket, name: &str) -> JoinHandle<()> {
    let report = report(name, "");
    tokio::spawn(async move {
        loop {
            let message = Message::binary(encode(&report).unwrap());
            if agent.send(message).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
}

#[tokio::test]
async fn drops_silent_agents_and_frees_their_name() {
    let server = TestServer::serve(quick_keepalive(app(&[], GeoIp::disabled()))).await;
    let reporter = keep_reporting(server.agent().await, "busy");
    // Like a client that vanished without closing its connection.
    let mut silent = server.agent().await;
    send_report(&mut silent, &report("quiet", "")).await;
    let mut dashboard = server.dashboard().await;
    poll_until(&mut dashboard, |nodes| nodes.len() == 2).await;

    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(expect_close(&mut silent).await, None);
    server.rejoin("quiet").await;
    // The client that kept reporting still holds its name.
    let mut impostor = server.agent().await;
    send_report(&mut impostor, &report("busy", "")).await;
    assert_eq!(expect_close(&mut impostor).await, Some(CloseCode::Policy));
    assert!(!reporter.is_finished());
    reporter.abort();
    server.stop().await;
}

#[tokio::test]
async fn pings_dashboards_and_drops_those_that_do_not_answer() {
    let server = TestServer::serve(quick_keepalive(app(&[], GeoIp::disabled()))).await;
    let mut live = server.dashboard().await;
    let mut gone = server.dashboard().await;
    assert!(poll(&mut live).await.is_empty());
    assert!(poll(&mut gone).await.is_empty());

    // Reading answers pings, as browsers do even when they stop polling.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    let mut pings = 0;
    while let Ok(message) = tokio::time::timeout_at(deadline, live.next()).await {
        match message {
            Some(Ok(Message::Ping(_))) => pings += 1,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(pings > 5, "{pings} pings");
    assert!(poll(&mut live).await.is_empty());
    // Never read from, so its pings went unanswered.
    assert_eq!(expect_close(&mut gone).await, None);
    server.stop().await;
}

#[tokio::test]
async fn geolocates_each_address_once() {
    let (ipinfo, lookups) = fake_ipinfo().await;
    let geoip = GeoIp::with_base_url(&ipinfo, None).unwrap();
    let server = TestServer::start(&[], geoip).await;
    let mut agent = server.agent().await;
    let mut dashboard = server.dashboard().await;

    // The connection address is not public here, so the reported one counts.
    for _ in 0..3 {
        send_report(&mut agent, &report("tokyo", "192.0.2.1")).await;
    }
    poll_until(&mut dashboard, |nodes| {
        nodes.first().is_some_and(|n| n.geoip == "JP")
    })
    .await;

    send_report(&mut agent, &report("tokyo", "198.51.100.1")).await;
    poll_until(&mut dashboard, |nodes| {
        nodes.first().is_some_and(|n| n.geoip == "US")
    })
    .await;
    assert_eq!(lookups.load(Ordering::SeqCst), 2);

    // Without any public address, the country stays as it was.
    send_report(&mut agent, &report("tokyo", "")).await;
    send_report(&mut agent, &report("marker", "")).await;
    let nodes = poll_until(&mut dashboard, |nodes| nodes.len() == 2).await;
    assert_eq!(nodes[1].geoip, "US");

    // Another node at a known address reuses its country.
    let mut osaka = server.agent().await;
    send_report(&mut osaka, &report("osaka", "192.0.2.1")).await;
    poll_until(&mut dashboard, |nodes| {
        nodes.iter().any(|n| n.name == "osaka" && n.geoip == "JP")
    })
    .await;
    assert_eq!(lookups.load(Ordering::SeqCst), 2);
    server.stop().await;
}

#[tokio::test]
async fn geolocates_the_address_a_client_connects_from() {
    let (ipinfo, _) = fake_ipinfo().await;
    let geoip = GeoIp::with_base_url(&ipinfo, None).unwrap();
    let server = TestServer::start(&[], geoip).await;
    // Behind a reverse proxy, which tells where the client connected from.
    let mut agent = server
        .agent_with(&[("x-forwarded-for", "203.0.113.9")])
        .await;
    send_report(&mut agent, &report("proxied", "")).await;
    let mut dashboard = server.dashboard().await;
    poll_until(&mut dashboard, |nodes| {
        nodes.first().is_some_and(|n| n.geoip == "DE")
    })
    .await;

    // It takes precedence over an address the client reports.
    let mut reported = report("proxied", "192.0.2.1");
    reported.data.stats.cpu_usage = 42.0;
    send_report(&mut agent, &reported).await;
    let nodes = poll_until(&mut dashboard, |nodes| nodes[0].stats.cpu_usage == 42.0).await;
    assert_eq!(nodes[0].geoip, "DE");
    server.stop().await;
}

#[tokio::test]
async fn serves_the_dashboard_from_a_directory() {
    let dir = tempfile::tempdir().unwrap();
    let index = format!(
        "<!doctype html><title>AIOM</title>{}",
        "<!-- padding -->".repeat(20)
    );
    std::fs::write(dir.path().join("index.html"), &index).unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/index-abc123.js"), "console.log(1)").unwrap();
    let server = TestServer::start(
        &["--web-dir", dir.path().to_str().unwrap()],
        GeoIp::disabled(),
    )
    .await;
    let client = reqwest_client();

    let page = client.get(server.url("/")).send().await.unwrap();
    assert_eq!(page.status(), 200);
    assert_eq!(page.headers()["cache-control"], "no-cache");
    assert_eq!(page.text().await.unwrap(), index);

    let asset = client
        .get(server.url("/assets/index-abc123.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(asset.status(), 200);
    assert_eq!(
        asset.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );

    let compressed = client
        .get(server.url("/"))
        .header("accept-encoding", "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(compressed.headers()["content-encoding"], "gzip");

    let missing = client.get(server.url("/missing.js")).send().await.unwrap();
    assert_eq!(missing.status(), 404);
    assert!(!missing.headers().contains_key("cache-control"));

    // The other routes stay as they are.
    let health = client.get(server.url(HEALTH_PATH)).send().await.unwrap();
    assert_eq!(health.text().await.unwrap(), "ok");
    let mut dashboard = server.dashboard().await;
    assert!(poll(&mut dashboard).await.is_empty());
    server.stop().await;
}

#[test]
fn web_dir_needs_an_index() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(&["--web-dir", dir.path().to_str().unwrap()]);
    let error = App::new(&config, GeoIp::disabled()).unwrap_err();
    assert!(error.to_string().contains("index.html"), "{error}");
}

#[tokio::test]
async fn closes_open_connections_on_shutdown() {
    let server = TestServer::default().await;
    let mut agent = server.agent().await;
    let mut dashboard = server.dashboard().await;
    // Make sure both connections are fully set up before shutting down.
    send_report(&mut agent, &report("node", "")).await;
    poll_until(&mut dashboard, |nodes| !nodes.is_empty()).await;

    server.stop().await;
    assert_eq!(expect_close(&mut agent).await, Some(CloseCode::Away));
    assert_eq!(expect_close(&mut dashboard).await, Some(CloseCode::Away));
}

fn reqwest_client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::new()
}

/// Starts a fake ipinfo.io and returns its URL and lookup counter.
async fn fake_ipinfo() -> (String, Arc<AtomicUsize>) {
    let lookups = Arc::new(AtomicUsize::new(0));
    let counter = lookups.clone();
    let app = axum::Router::new().route(
        "/{ip}/country",
        axum::routing::get(move |Path(ip): Path<String>| {
            counter.fetch_add(1, Ordering::SeqCst);
            async move {
                match ip.as_str() {
                    "192.0.2.1" => "JP",
                    "198.51.100.1" => "US",
                    "203.0.113.9" => "DE",
                    _ => "bogon",
                }
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    (url, lookups)
}

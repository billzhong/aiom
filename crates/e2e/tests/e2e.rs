//! Runs the real client against the real server and watches the result as the dashboard.

#![allow(clippy::unwrap_used)]

use aiom_client::{Collector, session};
use aiom_protocol::{DASHBOARD_PING, NodeSnapshot, decode};
use aiom_server::{App, GeoIp};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

const TOKEN: &str = "e2e-token";
const TIMEOUT: Duration = Duration::from_secs(15);

struct Running {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Running {
    async fn stop(self) {
        self.stop.send(()).unwrap();
        timeout(TIMEOUT, self.task)
            .await
            .expect("did not stop in time")
            .unwrap();
    }
}

async fn start_server(listener: TcpListener) -> Running {
    let config = aiom_server::Config::try_parse_from(["aiom-server", "--token", TOKEN]).unwrap();
    let app = App::new(&config, GeoIp::disabled())
        .unwrap()
        .with_snapshot_ttl(Duration::ZERO);
    let (stop, signal) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        app.serve(listener, async {
            signal.await.ok();
        })
        .await
        .unwrap();
    });
    Running { stop, task }
}

fn start_client(name: &str, addr: SocketAddr, token: &str) -> Running {
    let url = Url::parse(&format!("ws://{addr}/agent")).unwrap();
    let config = session::Config::new(
        name,
        &url,
        token,
        Duration::from_millis(100),
        Duration::from_millis(500),
    )
    .unwrap();
    let (stop, signal) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let mut collector = Collector::new();
        session::run(&config, &mut collector, async {
            signal.await.ok();
        })
        .await;
    });
    Running { stop, task }
}

async fn poll(addr: SocketAddr) -> Vec<NodeSnapshot> {
    let (mut dashboard, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
    dashboard.send(Message::text(DASHBOARD_PING)).await.unwrap();
    loop {
        let message = timeout(TIMEOUT, dashboard.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Binary(payload) = message {
            return decode(&payload, 1 << 20).unwrap();
        }
    }
}

async fn wait_for_node(addr: SocketAddr, name: &str) -> NodeSnapshot {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(node) = poll(addr).await.into_iter().find(|n| n.name == name) {
            return node;
        }
        assert!(Instant::now() < deadline, "{name} never showed up");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn client_reports_show_up_on_the_dashboard() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = start_server(listener).await;
    let client = start_client("e2e-node", addr, TOKEN);

    let node = wait_for_node(addr, "e2e-node").await;
    assert_eq!(node.host.agent, env!("CARGO_PKG_VERSION"));
    assert!(node.host.total_memory > 0);
    assert!(node.stats.process_count > 0);
    assert!(node.stats.uptime > 0);

    // Reports keep coming: the stored timestamp follows the server clock.
    let first = node.timestamp;
    let deadline = Instant::now() + TIMEOUT;
    while wait_for_node(addr, "e2e-node").await.timestamp == first {
        assert!(Instant::now() < deadline, "no new report arrived");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    client.stop().await;
    server.stop().await;
}

#[tokio::test]
async fn client_with_wrong_token_is_ignored() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = start_server(listener).await;
    let intruder = start_client("intruder", addr, "wrong-token");
    let legit = start_client("legit", addr, TOKEN);

    wait_for_node(addr, "legit").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let names: Vec<_> = poll(addr).await.into_iter().map(|n| n.name).collect();
    assert_eq!(names, ["legit"]);

    intruder.stop().await;
    legit.stop().await;
    server.stop().await;
}

#[tokio::test]
async fn client_reconnects_after_server_restart() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = start_server(listener).await;
    let client = start_client("phoenix", addr, TOKEN);
    wait_for_node(addr, "phoenix").await;

    server.stop().await;
    // The new server starts empty, so the node only reappears if the client reconnects.
    let server = start_server(TcpListener::bind(addr).await.unwrap()).await;
    wait_for_node(addr, "phoenix").await;

    client.stop().await;
    server.stop().await;
}

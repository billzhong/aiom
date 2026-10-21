//! Runs the reporting loop against a fake server.

#![allow(clippy::unwrap_used)]

use aiom_client::{Collector, session};
use aiom_protocol::{AgentReport, PROTOCOL_VERSION, decode, is_public};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::{Message, http};
use tokio_tungstenite::{WebSocketStream, accept_hdr_async};
use url::Url;

const TIMEOUT: Duration = Duration::from_secs(10);

struct Connection {
    authorization: Option<String>,
    socket: WebSocketStream<TcpStream>,
}

/// Accepts WebSocket connections and hands them over with their `Authorization` header.
async fn fake_server() -> (Url, mpsc::UnboundedReceiver<Connection>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("ws://{}/agent", listener.local_addr().unwrap())).unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let mut authorization = None;
            // The signature, including its large error type, is defined by tungstenite.
            #[allow(clippy::result_large_err)]
            let callback = |request: &Request, response: Response| {
                authorization = request
                    .headers()
                    .get(http::header::AUTHORIZATION)
                    .map(|v| v.to_str().unwrap().to_owned());
                Ok(response)
            };
            if let Ok(socket) = accept_hdr_async(stream, callback).await {
                let _ = tx.send(Connection {
                    authorization,
                    socket,
                });
            }
        }
    });
    (url, rx)
}

struct Client {
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Client {
    fn start(url: &Url, retry: Duration) -> Self {
        let config = session::Config::new(
            "test-node",
            url,
            "s3cret",
            Duration::from_millis(100),
            retry,
        )
        .unwrap();
        let (shutdown, signal) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let mut collector = Collector::new();
            session::run(&config, &mut collector, async {
                signal.await.ok();
            })
            .await;
        });
        Self { shutdown, task }
    }

    async fn stop(self) {
        self.shutdown.send(()).unwrap();
        timeout(TIMEOUT, self.task)
            .await
            .expect("client did not stop")
            .unwrap();
    }
}

async fn next_connection(connections: &mut mpsc::UnboundedReceiver<Connection>) -> Connection {
    timeout(TIMEOUT, connections.recv())
        .await
        .expect("client did not connect")
        .unwrap()
}

async fn next_message(socket: &mut WebSocketStream<TcpStream>) -> Message {
    timeout(TIMEOUT, socket.next())
        .await
        .expect("client sent nothing")
        .expect("connection ended")
        .unwrap()
}

#[tokio::test]
async fn reports_periodically_with_token() {
    let (url, mut connections) = fake_server().await;
    let client = Client::start(&url, Duration::from_secs(1));

    let mut connection = next_connection(&mut connections).await;
    assert_eq!(connection.authorization.as_deref(), Some("Bearer s3cret"));

    let started = Instant::now();
    for _ in 0..3 {
        let Message::Binary(payload) = next_message(&mut connection.socket).await else {
            panic!("expected a binary report");
        };
        let report: AgentReport = decode(&payload, 1 << 20).unwrap();
        assert_eq!(report.version, PROTOCOL_VERSION);
        assert_eq!(report.data.name, "test-node");
        assert_eq!(report.data.host.agent, env!("CARGO_PKG_VERSION"));
        assert!(report.data.host.total_memory > 0);
        assert!(report.timestamp > 1_700_000_000);
        for ip in [&report.data.ip.public_ipv4, &report.data.ip.public_ipv6] {
            assert!(ip.is_empty() || is_public(ip.parse().unwrap()), "{ip}");
        }
    }
    // The first report goes out right away, the others one interval apart.
    assert!(started.elapsed() >= Duration::from_millis(150));

    client.stop().await;
    loop {
        match next_message(&mut connection.socket).await {
            Message::Close(_) => break,
            Message::Binary(_) => continue,
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn reconnects_after_server_closes_connection() {
    let (url, mut connections) = fake_server().await;
    let client = Client::start(&url, Duration::from_secs(1));

    let mut first = next_connection(&mut connections).await;
    next_message(&mut first.socket).await;
    first.socket.close(None).await.unwrap();

    let mut second = next_connection(&mut connections).await;
    assert!(matches!(
        next_message(&mut second.socket).await,
        Message::Binary(_)
    ));
    client.stop().await;
}

#[tokio::test]
async fn keeps_retrying_an_unreachable_server_until_shutdown() {
    let unused = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = Url::parse(&format!("ws://{}/agent", unused.local_addr().unwrap())).unwrap();
    drop(unused);

    let client = Client::start(&url, Duration::from_secs(60));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!client.task.is_finished());
    // Shutdown interrupts the retry delay.
    let started = Instant::now();
    client.stop().await;
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn rejects_tokens_that_are_not_header_values() {
    let url = Url::parse("ws://127.0.0.1/agent").unwrap();
    let interval = Duration::from_secs(1);
    assert!(session::Config::new("n", &url, "line\nbreak", interval, interval).is_err());
}

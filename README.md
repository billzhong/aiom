<div align="center">

# AIO Monitor

**A lightweight, real-time monitoring system for your servers.**

A tiny agent on every host, one in-memory server, and a live web dashboard. No database to
run, no root required.

[![CI](https://github.com/billzhong/aiom/actions/workflows/ci.yml/badge.svg)](https://github.com/billzhong/aiom/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/billzhong/aiom)](https://github.com/billzhong/aiom/releases)
[![Docker](https://img.shields.io/badge/ghcr.io-billzhong%2Faiom-blue?logo=docker&logoColor=white)](https://github.com/billzhong/aiom/pkgs/container/aiom)
[![MSRV](https://img.shields.io/badge/rust-1.95%2B-orange?logo=rust)](Cargo.toml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

[Features](#features) · [Quick start](#quick-start) · [Installation](#installation) ·
[Configuration](#configuration) · [Deployment](#deployment) · [Development](#development)

</div>

---

## Features

- **Real time**: hosts report every second by default, and the dashboard refreshes every second.
- **Tiny agent**: a single static binary built for size, which needs no privileges to read
  system metrics. Runs on Linux, macOS and Windows.
- **Zero state**: the server keeps nodes in memory only. No database to run, back up or migrate.
- **Rich metrics**: CPU, memory, swap, disk, network traffic and speed, load average, TCP and
  UDP connections, processes and uptime, plus OS, CPU model, architecture and virtualization.
- **Geolocation**: nodes are placed on a country by the address they connect from, through
  [ipinfo.io](https://ipinfo.io).
- **Modern dashboard**: built with React, Tailwind CSS and shadcn/ui. Per-node history charts,
  filtering by country and name, light and dark themes, English and Chinese.
- **Deploy anywhere**: one multi-arch Docker image that serves the dashboard too, prebuilt
  binaries, or the dashboard on any static host.
- **Secure by default**: agents authenticate with a bearer token. The dashboard channel is
  read-only.

## How it works

```
┌────────────────────┐  wss://…/agent (token)   ┌──────────────────┐   wss://…/ws    ┌───────────┐
│ aiom-client        │ ───────────────────────▶ │   aiom-server    │ ◀─────────────▶ │ dashboard │
│  (every host)      │   report every second    │    (in memory)   │  poll each sec  │  (React)  │
└────────────────────┘                          └──────────────────┘                 └───────────┘
```

`aiom-client` collects the metrics of the host it runs on and streams them to `aiom-server`
over a WebSocket. The server keeps the latest state of every node in memory, looks up where
each node is, and serves the node list to dashboards over a second WebSocket. The server can
serve the dashboard itself, or the dashboard can be hosted anywhere else.

## Quick start

Start the server and dashboard with Docker:

```sh
docker run -d --name aiom -p 3000:3000 -e AIOM_TOKEN=change-me ghcr.io/billzhong/aiom:latest
```

Connect a host to it, using a client binary from the
[latest release](https://github.com/billzhong/aiom/releases/latest):

```sh
aiom-client --name my-laptop --server ws://127.0.0.1:3000/agent --token change-me
```

Then open <http://localhost:3000>.

> [!IMPORTANT]
> Use a long random token, e.g. from `openssl rand -hex 32`. In production, put the server
> behind a reverse proxy with TLS so clients connect over `wss://`. See
> [Deployment](#deployment).

## Installation

### Docker (server)

Images for `linux/amd64` and `linux/arm64` are published to
[`ghcr.io/billzhong/aiom`](https://github.com/billzhong/aiom/pkgs/container/aiom):

| Tag            | Built from                |
| -------------- | ------------------------- |
| `latest`       | The latest release        |
| `X.Y.Z`, `X.Y` | The matching release      |
| `main`         | Every push to `main`      |
| `sha-<commit>` | The commit, for pinning   |

The image runs the server on a distroless base and serves the dashboard on port 3000. With
Docker Compose, from a clone of this repository:

```sh
AIOM_TOKEN=change-me docker compose up -d
```

### Prebuilt binaries

Each [release](https://github.com/billzhong/aiom/releases) has the following archives, each
with a `.sha256` checksum:

| Archive                                         | Platform                       |
| ----------------------------------------------- | ------------------------------ |
| `aiom-client-x86_64-unknown-linux-musl.tar.gz`  | Linux x86_64 (static)          |
| `aiom-client-aarch64-unknown-linux-musl.tar.gz` | Linux aarch64 (static)         |
| `aiom-client-x86_64-unknown-linux-gnu.tar.gz`   | Linux x86_64 (glibc)           |
| `aiom-client-x86_64-pc-windows-msvc.zip`        | Windows x86_64                 |
| `aiom-client-aarch64-apple-darwin.tar.gz`       | macOS (Apple silicon)          |
| `aiom-server-<target>.tar.gz`                   | The same Linux targets         |
| `aiom-web.tar.gz`                               | Built dashboard (static files) |

For example, to install the client on a Linux x86_64 host:

```sh
curl -fsSL https://github.com/billzhong/aiom/releases/latest/download/aiom-client-x86_64-unknown-linux-musl.tar.gz | tar xz
sudo install -m 755 aiom-client-x86_64-unknown-linux-musl/aiom-client /usr/local/bin/
```

Linux archives include a systemd unit as well, see [Running with systemd](#running-with-systemd).

### From source

Requires Rust 1.95 or newer, and for the dashboard Node.js 22.12 or newer with pnpm.

```sh
git clone https://github.com/billzhong/aiom.git && cd aiom
cargo build --release              # target/release/aiom-server and aiom-client
cd web && pnpm install && pnpm build   # web/dist
```

## Configuration

### Server

The server is configured through environment variables, a `.env` file in the working directory
(see [`.env.example`](.env.example)), or the equivalent flags (`aiom-server --help`).

| Variable              | Flag               | Default        | Description                                                  |
| --------------------- | ------------------ | -------------- | ------------------------------------------------------------ |
| `AIOM_TOKEN`          | `--token`          | _(required)_   | Token clients authenticate with                              |
| `AIOM_LISTEN`         | `--listen`         | `0.0.0.0:3000` | Address to listen on                                         |
| `AIOM_DASHBOARD_PATH` | `--dashboard-path` | `/ws`          | Dashboard WebSocket route (public, read-only)                |
| `AIOM_AGENT_PATH`     | `--agent-path`     | `/agent`       | Client WebSocket route (requires `Authorization: Bearer`)    |
| `AIOM_IPINFO_TOKEN`   | `--ipinfo-token`   | _(unset)_      | ipinfo.io token, for geolocation beyond the anonymous limit  |
| `AIOM_WEB_DIR`        | `--web-dir`        | _(unset)_      | Built dashboard to serve on all other routes                 |
| `RUST_LOG`            |                    | `info`         | Log filter, e.g. `debug` or `aiom_server=debug,info`         |

`GET /healthz` answers `ok` while the server is running.

Nodes are kept in memory only: after a restart, the list fills up again as clients reconnect.
Each node name can be used by one client at a time. While a node is connected, the server
refuses other clients reporting under its name.

### Client

```
aiom-client -n <name> -s <wss://host/agent> -t <token> [--interval 1000] [--retry 5] [--debug]
```

| Flag           | Environment     | Default | Description                                   |
| -------------- | --------------- | ------- | --------------------------------------------- |
| `-n, --name`   | `AIOM_NAME`     |         | Name of this node on the dashboard (unique)   |
| `-s, --server` | `AIOM_SERVER`   |         | Agent URL of the server (`ws://` or `wss://`) |
| `-t, --token`  | `AIOM_TOKEN`    |         | Token configured on the server                |
| `--interval`   | `AIOM_INTERVAL` | `1000`  | Report interval in ms, 500–5000               |
| `--retry`      | `AIOM_RETRY`    | `5`     | Reconnect delay in seconds, 1–60              |
| `--debug`      |                 |         | Verbose logging                               |

> [!TIP]
> Prefer `AIOM_TOKEN` over `--token`: command lines are visible to every user of the host.

### Dashboard

Without configuration, the dashboard connects to `/ws` on the host serving the page. To host it
somewhere else, build it with `VITE_WS_URL` set to the dashboard URL of the server:

```sh
cd web
VITE_WS_URL=wss://monitor.example.com/ws pnpm build
```

`VITE_WS_URL` can also be set in `web/.env`, or in `web/.env.production.local` or
`web/.env.development.local` to limit it to `pnpm build` or `pnpm dev`, see
[`web/.env.example`](web/.env.example). None of these files are committed. If you changed
`AIOM_DASHBOARD_PATH`, set `VITE_WS_URL` to the new path even when the server serves the
dashboard.

## Deployment

### Behind a reverse proxy

Terminate TLS at a reverse proxy, so clients and dashboards can use `wss://`. Nodes are
geolocated by the address they connect from, so the proxy needs to pass it on in
`X-Forwarded-For` or `X-Real-IP`. Without it, the server falls back to the public addresses the
clients report, if any.

With [Caddy](https://caddyserver.com), which handles TLS, WebSockets and `X-Forwarded-For` by
default:

```caddyfile
monitor.example.com {
    reverse_proxy 127.0.0.1:3000
}
```

With nginx:

```nginx
location / {
    proxy_pass http://127.0.0.1:3000;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
}
```

### Running with systemd

[`deploy/`](deploy) has hardened systemd units for both binaries, which run as a dynamic,
unprivileged user. To set up a client:

```sh
sudo install -m 755 aiom-client /usr/local/bin/
sudo install -D -m 600 /dev/stdin /etc/aiom/client.env <<'END'
AIOM_NAME=web-1
AIOM_SERVER=wss://monitor.example.com/agent
AIOM_TOKEN=change-me
END
sudo install -m 644 aiom-client.service /etc/systemd/system/
sudo systemctl enable --now aiom-client
```

The server works the same way with `/etc/aiom/server.env`, see the comments in
[`deploy/aiom-server.service`](deploy/aiom-server.service). To have it serve the dashboard,
extract `aiom-web.tar.gz` to a directory and point `AIOM_WEB_DIR` at it.

## Protocol

Both channels carry gzip-compressed JSON in binary WebSocket frames.

- **Agent channel**: the client sends `{"version": 1, "timestamp", "data": {"name", "host",
  "stats", "ip"}}` every interval, where `ip` holds the public addresses of the interfaces the
  host reaches the internet from (empty behind NAT). The server geolocates the address the
  client connects from, and falls back to `ip` when that one is not public. It closes the
  connection with code 1002 for other versions, 1007 for malformed messages and 1008 while
  another client is connected under the same name.
- **Dashboard channel**: the dashboard sends the text frame `ping`, and the server answers with
  an array of `{"timestamp", "name", "host", "stats", "geoip"}`, sorted by name. `timestamp` is
  when the server last heard from the node.
- **Both channels**: the server pings every 5 seconds, and drops connections on which nothing
  arrived for 20 seconds, pongs included.

## Development

### Project layout

| Path              | Contents                                                                |
| ----------------- | ----------------------------------------------------------------------- |
| `crates/server`   | `aiom-server`: receives reports, geolocates nodes, serves the node list |
| `crates/client`   | `aiom-client`: collects metrics of the host it runs on                  |
| `crates/protocol` | Wire format shared by server and client                                 |
| `crates/e2e`      | End-to-end tests running the real client against the real server       |
| `web`             | Dashboard (React, Vite, Tailwind CSS, shadcn/ui)                        |
| `deploy`          | systemd units                                                           |

### Running locally

```sh
# Server
AIOM_TOKEN=change-me cargo run -p aiom-server

# Client, in another terminal
cargo run -p aiom-client -- -n my-laptop -s ws://127.0.0.1:3000/agent -t change-me

# Dashboard on http://localhost:5173, in a third terminal. It proxies /ws to the server above.
cd web && pnpm install && pnpm dev
```

### Checks

These are the checks CI runs, so run them before opening a pull request:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd web
pnpm format:check && pnpm lint && pnpm typecheck && pnpm test && pnpm build
```

The minimum supported Rust version is set by `rust-version` in [`Cargo.toml`](Cargo.toml). The
dashboard uses the pnpm version set by `packageManager` in [`web/package.json`](web/package.json),
Oxlint for linting and Oxfmt for formatting.

### Continuous integration

- **[CI](.github/workflows/ci.yml)**, on pushes and pull requests: formatting, Clippy, tests on
  Linux, macOS and Windows, an MSRV build, `cargo deny` (advisories, licenses, sources), the
  web checks, and a Docker build with a smoke test. Jobs only run when their part of the
  repository changed. The final `CI` job is the one to require in branch protection.
- **[Release](.github/workflows/release.yml)** builds all binaries and the dashboard on every
  push to `main`, and publishes them as a GitHub release when a `vX.Y.Z` tag matching the crate
  version is pushed.
- **[Docker](.github/workflows/docker.yml)** builds the server image natively on amd64 and arm64
  runners, and pushes a multi-platform image to GHCR.
- Dependabot keeps Cargo, pnpm, GitHub Actions and Docker dependencies up to date.

## Contributing

Bug reports, feature requests and pull requests are welcome. For larger changes, please
[open an issue](https://github.com/billzhong/aiom/issues) first to discuss what you would like
to change. Make sure the [checks](#checks) pass, and add tests for new behavior.

## License

AIO Monitor is released under the [MIT License](LICENSE).

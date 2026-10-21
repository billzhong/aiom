# AIO Monitor

A lightweight monitoring system for servers. A small client runs on every monitored host and
reports its metrics to a server, which keeps the latest state of every node in memory and
streams it to a web dashboard. The server can serve the dashboard itself, or the dashboard can
be hosted anywhere else.

```
┌────────────────────┐  wss://…/agent (token)   ┌──────────────────┐   wss://…/ws    ┌───────────┐
│ aiom-client        │ ───────────────────────▶ │   aiom-server    │ ◀─────────────▶ │ dashboard │
│  (every host)      │   report every second    │    (in memory)   │  poll each sec  │  (React)  │
└────────────────────┘                          └──────────────────┘                 └───────────┘
```

| Path              | What                                                                    |
| ----------------- | ----------------------------------------------------------------------- |
| `crates/server`   | `aiom-server`: receives reports, geolocates nodes, serves the node list |
| `crates/client`   | `aiom-client`: collects metrics of the host it runs on                  |
| `crates/protocol` | Wire format shared by server and client                                 |
| `crates/e2e`      | End-to-end tests running the real client against the real server       |
| `web`             | Dashboard (React, Vite, Tailwind CSS, shadcn/ui)                        |
| `deploy`          | systemd units                                                           |

Reported metrics: CPU usage, memory, swap, disk, network traffic and speed, load average, TCP
and UDP connections, process count, uptime, plus host facts (OS, CPU, architecture,
virtualization) and the country of the address the client connects from.

## Quick start

```sh
# Server
AIOM_TOKEN=change-me cargo run -p aiom-server

# Client, in another terminal
cargo run -p aiom-client -- -n my-laptop -s ws://127.0.0.1:3000/agent -t change-me

# Dashboard, in a third terminal: http://localhost:5173, connected to the server above
cd web && pnpm install && pnpm dev
```

## Server

Configured through environment variables, a `.env` file in the working directory (see
[`.env.example`](.env.example)), or the equivalent flags (`aiom-server --help`).

| Variable              | Flag               | Default        | Description                                                |
| --------------------- | ------------------ | -------------- | ---------------------------------------------------------- |
| `AIOM_TOKEN`          | `--token`          | _(required)_   | Token clients authenticate with                            |
| `AIOM_LISTEN`         | `--listen`         | `0.0.0.0:3000` | Address to listen on                                       |
| `AIOM_DASHBOARD_PATH` | `--dashboard-path` | `/ws`          | Dashboard WebSocket route (public, read-only)              |
| `AIOM_AGENT_PATH`     | `--agent-path`     | `/agent`       | Client WebSocket route (requires `Authorization: Bearer`)  |
| `AIOM_IPINFO_TOKEN`   | `--ipinfo-token`   | _(unset)_      | ipinfo.io token for geolocation beyond the anonymous limit |
| `AIOM_WEB_DIR`        | `--web-dir`        | _(unset)_      | Built dashboard to serve on all other routes, see below    |
| `RUST_LOG`            |                    | `info`         | Log filter                                                 |

`GET /healthz` answers `ok` while the server runs. Put the server behind a TLS-terminating
reverse proxy so clients and dashboards can use `wss://`, and have the proxy pass the client
address in `X-Forwarded-For` or `X-Real-IP`, as most do by default: nodes are geolocated by the
address they connect from, or else by the public addresses their clients report.

Nodes are kept in memory only: after a restart, the list fills up again as clients reconnect.
Each node name can be used by one client at a time; while a node is connected, the server
refuses other clients reporting under its name.

### Docker

```sh
AIOM_TOKEN=change-me docker compose up -d
```

The image serves the dashboard as well, on http://localhost:3000. Images for `linux/amd64` and
`linux/arm64` are published to `ghcr.io/<owner>/<repo>` on every push to `main` (tag `main`) and
for every release (tags `X.Y.Z`, `X.Y` and `latest`).

## Client

```
aiom-client -n <name> -s <wss://host/agent> -t <token> [--interval 1000] [--retry 5] [--debug]
```

| Flag           | Environment     | Description                                    |
| -------------- | --------------- | ---------------------------------------------- |
| `-n, --name`   | `AIOM_NAME`     | Name of this node on the dashboard (unique)    |
| `-s, --server` | `AIOM_SERVER`   | Agent URL of the server (`ws://` or `wss://`)  |
| `-t, --token`  | `AIOM_TOKEN`    | Token configured on the server                 |
| `--interval`   | `AIOM_INTERVAL` | Report interval in ms, 500–5000 (default 1000) |
| `--retry`      | `AIOM_RETRY`    | Reconnect delay in seconds, 1–60 (default 5)   |
| `--debug`      |                 | Verbose logging                                |

Prefer the environment variables for the token: command lines are visible to every user of the
host. [`deploy/aiom-client.service`](deploy/aiom-client.service) shows a systemd setup.
Prebuilt binaries for Linux (x86_64 and aarch64), Windows (x86_64) and macOS (Apple silicon) are
attached to each [release](../../releases).

## Dashboard

`pnpm build` puts the dashboard in `web/dist`; each release has it attached as
`aiom-web.tar.gz`. There are two ways to deploy it:

- **Served by the server**: point `AIOM_WEB_DIR` at the directory, as the Docker image does.
  The dashboard then connects to `/ws` on the same server, so nothing else needs configuring.
- **Hosted separately** on any static host: build it with `VITE_WS_URL` set to the dashboard
  URL of the server, e.g. `wss://monitor.example.com/ws`, see
  [`web/.env.example`](web/.env.example). Put it in `web/.env`, in
  `web/.env.production.local` or `web/.env.development.local` to limit it to `pnpm build` or
  `pnpm dev` (none of them is committed), or pass it as an environment variable, which takes
  precedence.

Without `VITE_WS_URL`, the dashboard connects to `/ws` on the host serving the page; with a
custom `AIOM_DASHBOARD_PATH`, set `VITE_WS_URL` to that path. `pnpm dev` passes `/ws` on to a
server on `localhost:3000`.

## Development

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

### CI

- **CI** (`ci.yml`, pushes and pull requests): formatting, Clippy, tests on Linux, macOS and
  Windows, an MSRV build, `cargo deny` (advisories, licenses, sources), the web checks, and a
  Docker build with a smoke test. Jobs only run when their part of the repository changed; the
  final `CI` job is the one to require in branch protection.
- **Release** (`release.yml`): builds all binaries and the web UI on every push to `main`, and
  publishes them as a GitHub release when a `vX.Y.Z` tag matching the crate version is pushed.
- **Docker** (`docker.yml`): builds the server image natively on amd64 and arm64 runners and
  pushes a multi-platform image to GHCR.
- Dependabot keeps Cargo, pnpm, GitHub Actions and Docker dependencies up to date.

## Protocol

Both channels carry gzip-compressed JSON in binary WebSocket frames.

- **Agent channel**: the client sends `{"version": 1, "timestamp", "data": {"name", "host",
  "stats", "ip"}}` every interval, where `ip` holds the public addresses of the interfaces the
  host reaches the internet from (empty behind NAT). The server geolocates the address the
  client connects from, and falls back to `ip` when that one is not public. It closes the
  connection with code 1002 for other versions, 1007 for malformed messages and 1008 while
  another client is connected under the same name.
- **Both channels**: the server pings every 5 seconds, and drops connections on which nothing
  arrived for 20 seconds, pongs included.
- **Dashboard channel**: the dashboard sends the text frame `ping`; the server answers with an
  array of `{"timestamp", "name", "host", "stats", "geoip"}`, sorted by name. `timestamp` is
  when the server last heard from the node.

## License

[MIT](LICENSE)

# syntax=docker/dockerfile:1

# The AIO Monitor server as a static musl binary on a distroless base image, serving the
# dashboard as well.

# The dashboard is the same on every platform, so it is built once, natively.
FROM --platform=$BUILDPLATFORM node:24-alpine AS web
WORKDIR /web
COPY web/package.json web/pnpm-lock.yaml ./
RUN npm install --global "$(node -p 'require("./package.json").packageManager')" \
    && pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm run build

FROM lukemathwalker/cargo-chef:latest-rust-alpine AS chef
WORKDIR /build

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
# Dependencies only, so this layer stays cached until Cargo.toml or Cargo.lock change.
RUN cargo chef cook --release --locked --package aiom-server --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo build --release --locked --package aiom-server

FROM gcr.io/distroless/static-debian13:nonroot
COPY --from=builder /build/target/release/aiom-server /usr/local/bin/aiom-server
COPY LICENSE /usr/share/doc/aiom/LICENSE
COPY --from=web /web/dist /usr/share/aiom/web
ENV AIOM_LISTEN=0.0.0.0:3000 \
    AIOM_WEB_DIR=/usr/share/aiom/web
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/aiom-server"]

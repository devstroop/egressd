FROM rust:1.82-slim-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY crates crates
COPY src src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/egressd /usr/local/bin/egressd
EXPOSE 9090
HEALTHCHECK --interval=30s --timeout=3s --retries=3 CMD curl -sf http://127.0.0.1:9090/v1/health | grep -q '"status"'
ENTRYPOINT ["/usr/local/bin/egressd"]

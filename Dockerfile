# syntax=docker/dockerfile:1

# Front: build estático servido pelo próprio backend (mesma origem, sem CORS).
FROM docker.io/library/node:24-slim AS front
WORKDIR /src
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN npm run build

# Backend. A lib usa rustls + ring, então basta o compilador C da imagem.
FROM docker.io/library/rust:1.97-slim-trixie AS back
WORKDIR /src
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY backend/ backend/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p wpp-backend \
    && cp target/release/wpp-backend /usr/local/bin/wpp-backend

FROM docker.io/library/debian:trixie-slim
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1000 wpp \
    && useradd --uid 1000 --gid wpp --no-create-home --shell /usr/sbin/nologin wpp \
    && mkdir /data && chown wpp:wpp /data
COPY --from=back /usr/local/bin/wpp-backend /usr/local/bin/wpp-backend
COPY --from=front /src/dist /app/frontend

# 0.0.0.0 só dentro do container; no host a porta fica em 127.0.0.1 (ver compose).
ENV WPP_DATA_DIR=/data \
    WPP_FRONT_DIR=/app/frontend \
    WPP_BIND=0.0.0.0 \
    WPP_PORT=8787 \
    RUST_LOG=info
USER wpp
VOLUME /data
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/wpp-backend"]

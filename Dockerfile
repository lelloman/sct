ARG ENGINE_IMAGE=sct-engine:local
FROM ${ENGINE_IMAGE} AS engine
FROM node:24.13.0-bookworm-slim AS frontend
WORKDIR /web
COPY sct/web/package*.json ./
RUN npm ci --no-audit --no-fund
COPY sct/web/ ./
RUN npm run build

FROM rust:1.96-bookworm AS builder
ARG TARGETARCH
RUN rustup toolchain install 1.96.0 --profile minimal --component clippy --component rustfmt
WORKDIR /work/sct
COPY --from=engine /libsimple_server_engine.so /opt/simple-server/libsimple_server_engine.so
COPY --from=engine /SHA256SUMS /opt/simple-server/SHA256SUMS
RUN cd /opt/simple-server && sha256sum -c SHA256SUMS && ln -s libsimple_server_engine.so libsimple_server_engine.so.1
ENV SIMPLE_SERVER_ENGINE_DIR=/opt/simple-server LD_LIBRARY_PATH=/opt/simple-server
COPY simple-server /work/simple-server
COPY sct /work/sct
RUN --mount=type=cache,id=sct-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=sct-rust196-${TARGETARCH},target=/work/sct/target,sharing=locked \
    cargo build --locked --release -p sct-server --bin sct-server && \
    mkdir -p /out && cp target/release/sct-server /out/sct-server

FROM debian:bookworm-slim AS runtime
ARG SIMPLE_SERVER_REV
LABEL org.opencontainers.image.simple-server-revision=$SIMPLE_SERVER_REV
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /out/sct-server /usr/local/bin/sct-server
COPY --from=builder /opt/simple-server/ /opt/simple-server/
COPY --from=frontend /web/dist /app/web/dist
ENV LD_LIBRARY_PATH=/opt/simple-server SCT_BIND=0.0.0.0:8080 SCT_WEB_DIR=/app/web/dist
EXPOSE 8080
USER 65532:65532
ENTRYPOINT ["sct-server"]

ARG ENGINE_IMAGE=sct-engine:local
FROM ${ENGINE_IMAGE} AS engine
FROM node:24.13.0-bookworm-slim AS node
FROM rust:1.96-bookworm
ARG SIMPLE_SERVER_REV
LABEL org.opencontainers.image.simple-server-revision=$SIMPLE_SERVER_REV
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx
RUN apt-get update && apt-get install -y --no-install-recommends python3 sqlite3 curl jq ripgrep xorriso util-linux && rm -rf /var/lib/apt/lists/*
COPY --from=engine /libsimple_server_engine.so /opt/simple-server/libsimple_server_engine.so
COPY --from=engine /SHA256SUMS /opt/simple-server/SHA256SUMS
RUN cd /opt/simple-server && sha256sum -c SHA256SUMS && ln -s libsimple_server_engine.so libsimple_server_engine.so.1
ENV SIMPLE_SERVER_ENGINE_DIR=/opt/simple-server LD_LIBRARY_PATH=/opt/simple-server
COPY simple-server /work/simple-server
WORKDIR /work/sct
COPY sct /work/sct
RUN cargo test --locked --workspace --all-features --no-run && cargo build --locked --workspace
RUN npm --prefix web ci --no-audit --no-fund && npm --prefix web run build && cd web && npx playwright install --with-deps chromium
ENV SCT_QUALIFICATION=1 SCT_TEST_ARTIFACTS=/artifacts CARGO_NET_OFFLINE=true
CMD ["bash", "tests/qualification/inside.sh"]

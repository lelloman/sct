FROM node:24.13.0-bookworm-slim AS node
FROM rust:1.96-bookworm
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx
RUN apt-get update && apt-get install -y --no-install-recommends python3 sqlite3 curl jq ripgrep xorriso util-linux && rm -rf /var/lib/apt/lists/*
WORKDIR /work/sct
COPY sct /work/sct
RUN cargo test --locked --workspace --all-features --no-run && cargo build --locked --workspace
RUN npm --prefix web ci --no-audit --no-fund && npm --prefix web run build && cd web && npx playwright install --with-deps chromium
ENV SCT_QUALIFICATION=1 SCT_TEST_ARTIFACTS=/artifacts CARGO_NET_OFFLINE=true
CMD ["bash", "tests/qualification/inside.sh"]

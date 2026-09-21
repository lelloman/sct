#!/usr/bin/env bash
set -euo pipefail
mkdir -p /artifacts
export CARGO_NET_OFFLINE=true
export SCT_TEST_RECOVERY_BINARY="$PWD/target/debug/sct-recover"
# Production accepts plaintext S3 only on loopback. Local tunnels preserve that
# restriction while the disposable backends live on an isolated Docker network.
UPSTREAM_HOST=minio LISTEN_HOST=127.0.0.1 LISTEN_PORT=19000 CONTROL_PORT=18474 python3 tests/qualification/proxy.py > /artifacts/direct-tunnel.log 2>&1 &
sct_direct=$!
UPSTREAM_HOST=proxy UPSTREAM_PORT=9001 LISTEN_HOST=127.0.0.1 LISTEN_PORT=19001 CONTROL_PORT=18475 python3 tests/qualification/proxy.py > /artifacts/fault-tunnel.log 2>&1 &
sct_fault=$!
trap 'kill "$sct_direct" "$sct_fault" 2>/dev/null || true; chmod -R a+rwX /artifacts' EXIT
for attempt in {1..50}; do
  if curl -fsS http://127.0.0.1:19000/minio/health/ready >/dev/null; then break; fi
  sleep .1
done
curl -fsS http://127.0.0.1:19000/minio/health/ready >/dev/null
# Capture only nonsecret host/tool/image facts; never dump environment variables.
{ uname -a; rustc --version; node --version; df -h /work /artifacts; } > /artifacts/environment.txt
if [[ "${SCT_QUALIFY_STAGE:-all}" == packaging ]]; then
  ./scripts/check-client 2>&1 | tee /artifacts/standalone-client.log
  exit
fi
if [[ "${SCT_QUALIFY_STAGE:-all}" == adapter ]]; then
  cargo test --locked -p sct-core s3_store::tests::real_s3_conformance -- --ignored --nocapture 2>&1 | tee /artifacts/adapter.log
  exit
fi
cargo test --locked -p sct-server --test qualification -- --ignored --nocapture --test-threads=1 2>&1 | tee /artifacts/qualification.log
cargo test --locked -p sct-core -p sct-server --tests -- --ignored --skip real_transfer_placements --skip million_entry_preview --skip qualification_ 2>&1 | tee /artifacts/integration.log
SCT_QUALIFY_TREES=1 cargo test --locked -p sct-server --test transfers -- --ignored --nocapture 2>&1 | tee /artifacts/placements.log
cargo test --locked -p sct-core --test foundation million_entry_preview -- --ignored --nocapture 2>&1 | tee /artifacts/namespace-scale.log
./scripts/test-e2e --grep-invert 'M2:|M3:|M5:' 2>&1 | tee /artifacts/browser.log
cp -R web/test-results /artifacts/browser-results
./scripts/check-client 2>&1 | tee /artifacts/standalone-client.log
cargo test --locked -p sct-core --test foundation qualification_ten_million -- --ignored --nocapture 2>&1 | tee /artifacts/reference-scale.log

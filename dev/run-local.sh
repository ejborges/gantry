#!/usr/bin/env bash
# Build and run Gantry locally with sample data, for development and for browser tests.
#   dev/run-local.sh            build, reset the dev database, seed, serve on http://127.0.0.1:8600
#   dev/run-local.sh --keep     same, but keep the existing dev database
# Sign in as admin@example.org / "correct horse battery staple".
set -euo pipefail
cd "$(dirname "$0")/.."
CFG=dev/gantry.dev.toml
PSQL="psql -h 127.0.0.1 -p ${GANTRY_TEST_PGPORT:-5433} -U postgres -q -At"

dev/test-db.sh start
if [ "${1:-}" != "--keep" ]; then
  $PSQL -c "DROP DATABASE IF EXISTS gantry_dev WITH (FORCE)" -c "CREATE DATABASE gantry_dev" postgres
  rm -rf target/dev-blobs
fi
[ -d web/node_modules ] || (cd web && npm ci --no-audit --no-fund)
[ -f web/dist/index.html ] || (cd web && npm run build)
[ -f docs/manual/_site/index.html ] || (cd web && npm run manual)
cargo build -p gantry-server
BIN=target/debug/gantry
$BIN --config $CFG migrate
if [ "${1:-}" != "--keep" ]; then
  printf 'correct horse battery staple' | $BIN --config $CFG admin create --email admin@example.org --name "Morgan Reyes"
  $BIN --config $CFG serve &
  PID=$!
  trap 'kill $PID 2>/dev/null || true' EXIT
  for _ in $(seq 1 50); do curl -fs http://127.0.0.1:8600/readyz >/dev/null && break; sleep 0.2; done
  python3 dev/seed.py http://127.0.0.1:8600
  echo "Gantry is running at http://127.0.0.1:8600 (admin@example.org / correct horse battery staple)"
  wait $PID
else
  exec $BIN --config $CFG serve
fi

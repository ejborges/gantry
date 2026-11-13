#!/usr/bin/env bash
# Build and run Gantry locally with sample data, for development and for browser tests.
#   dev/run-local.sh            build, reset the dev database, seed, serve on port 8600
#                               (GANTRY_BIND changes the address; in the Vagrant machine it
#                               also prints the address to use from your computer)
#   dev/run-local.sh --keep     same, but keep the existing dev database
# Sign in as admin@example.org / "correct horse battery staple".
set -euo pipefail
cd "$(dirname "$0")/.."
CFG=dev/gantry.dev.toml
# The port Gantry listens on here (GANTRY_BIND overrides the configuration file; the
# Vagrant machine sets it so that forwarded ports reach the server).
BIND="${GANTRY_BIND:-127.0.0.1:8600}"
LOCAL="http://127.0.0.1:${BIND##*:}"
# Where it is reached from your computer, when this is the Vagrant machine.
HOST_PORTS=/etc/gantry-dev/host-ports.env
PSQL="psql -h 127.0.0.1 -p ${GANTRY_TEST_PGPORT:-5433} -U postgres -q -At"

where_from_host() {
  [ -r "$HOST_PORTS" ] || return 0
  # shellcheck disable=SC1090
  ( . "$HOST_PORTS"
    [ -n "${GANTRY_HOST_PORT_APP:-}" ] && echo "From your computer: http://localhost:$GANTRY_HOST_PORT_APP${GANTRY_GUEST_IP:+  (or http://$GANTRY_GUEST_IP:${BIND##*:})}" )
}

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
  for _ in $(seq 1 50); do curl -fs "$LOCAL/readyz" >/dev/null && break; sleep 0.2; done
  python3 dev/seed.py "$LOCAL"
  echo "Gantry is running at $LOCAL (admin@example.org / correct horse battery staple)"
  where_from_host
  wait $PID
else
  where_from_host
  exec $BIN --config $CFG serve
fi

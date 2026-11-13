#!/usr/bin/env bash
# Start (or stop) a throwaway PostgreSQL for tests and local development.
#   dev/test-db.sh start   -> listens on 127.0.0.1:5433, trust auth, superuser "postgres"
#   dev/test-db.sh stop
# Tests connect with GANTRY_TEST_DATABASE_URL (default postgres://postgres@127.0.0.1:5433/postgres).
# Data lives in $GANTRY_TEST_PGDATA (default: a directory under the system temp dir) and is disposable.
set -euo pipefail

PORT="${GANTRY_TEST_PGPORT:-5433}"
BIN="$(dirname "$(command -v initdb 2>/dev/null || ls -d /usr/pgsql-*/bin/initdb /usr/lib/postgresql/*/bin/initdb 2>/dev/null | sort -V | tail -1)")"
[ -x "$BIN/initdb" ] || { echo "PostgreSQL server binaries not found; install postgresql-server" >&2; exit 1; }

# PostgreSQL refuses to run as root, so drop to the postgres account when invoked as root.
if [ "$(id -u)" = 0 ]; then
  DATA="${GANTRY_TEST_PGDATA:-/var/lib/postgresql/gantry-dev}"
  [ -d /var/lib/postgresql ] || DATA="${GANTRY_TEST_PGDATA:-/var/lib/pgsql/gantry-dev}"
  mkdir -p "$DATA" && chown postgres: "$DATA"
  run() { su postgres -s /bin/sh -c "$*"; }
else
  DATA="${GANTRY_TEST_PGDATA:-${TMPDIR:-/tmp}/gantry-test-pg}"
  mkdir -p "$DATA"
  run() { sh -c "$*"; }
fi

case "${1:-start}" in
  start)
    if run "$BIN/pg_ctl -D $DATA status" >/dev/null 2>&1; then echo "already running on port $PORT"; exit 0; fi
    [ -f "$DATA/PG_VERSION" ] || run "$BIN/initdb -D $DATA -A trust -U postgres -E UTF8 --locale=C.UTF-8" >/dev/null
    rm -f "$DATA/postmaster.pid"
    run "$BIN/pg_ctl -D $DATA -w -l $DATA/server.log -o '-p $PORT -c listen_addresses=127.0.0.1 -c unix_socket_directories=/tmp -c fsync=off -c max_connections=300' start" >/dev/null
    echo "PostgreSQL $("$BIN"/postgres --version | awk '{print $3}') listening on 127.0.0.1:$PORT"
    ;;
  stop)
    run "$BIN/pg_ctl -D $DATA -m fast stop" || true
    ;;
  *) echo "usage: $0 start|stop" >&2; exit 2 ;;
esac

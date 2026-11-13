#!/usr/bin/bash
# Rehearses the upgrade from Gantry 0.1.0 on a real database: an installation is created and
# filled by the 0.1.0 server (taken from its RPM), stopped, migrated and started by the server
# from this tree, and then checked: is everything still there, and did what the upgrade notes
# promise happen?
#
#   packaging/tests/upgrade.sh PATH/TO/gantry-0.1.0-1.*.x86_64.rpm [NEW_GANTRY_BINARY]
#
# NEW_GANTRY_BINARY defaults to target/release/gantry or target/debug/gantry, whichever
# was built last.
# Needs rpm2cpio, cpio, psql, curl, python3 and a PostgreSQL to create a scratch database
# in: GANTRY_TEST_DATABASE_URL, by default the one dev/test-db.sh starts. Everything else
# happens in a temporary directory; the scratch database is dropped at the end.
#
# What this does not rehearse: the packages' own scriptlets and services (see
# packaging/tests/nginx-site.sh for the nginx part, and test the rest on Enterprise Linux).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
[ $# -ge 1 ] || { sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
old_rpm="$(readlink -f "$1")"
new="${2:-}"
if [ -z "$new" ]; then # whichever of the two was built last
  for c in "$repo/target/release/gantry" "$repo/target/debug/gantry"; do [ -x "$c" ] && { [ -z "$new" ] || [ "$c" -nt "$new" ]; } && new="$c"; done
fi
[ -x "$new" ] || { echo "no new gantry binary: build one with cargo build, or name it as the second argument" >&2; exit 2; }
for tool in rpm2cpio cpio psql curl python3; do command -v "$tool" >/dev/null || { echo "SKIP: $tool is not installed"; exit 0; }; done

admin_url="${GANTRY_TEST_DATABASE_URL:-postgres://postgres@127.0.0.1:5433/postgres}"
psql "$admin_url" -q -c "SELECT 1" >/dev/null 2>&1 || { echo "no PostgreSQL answers at $admin_url: start one with dev/test-db.sh start, or set GANTRY_TEST_DATABASE_URL" >&2; exit 1; }
db="gantry_upgrade_$$"
db_url="${admin_url%/*}/$db"
port="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
base="http://127.0.0.1:$port"
T="$(mktemp -d)"
server=""
cleanup() {
  if [ -n "$server" ]; then kill "$server" 2>/dev/null || true; fi
  wait 2>/dev/null || true
  psql "$admin_url" -q -c "DROP DATABASE IF EXISTS $db WITH (FORCE)" >/dev/null 2>&1 || true
  rm -rf "$T"
}
trap cleanup EXIT

mkdir -p "$T/old" "$T/blobs" "$T/addons" "$T/web"
(cd "$T/old" && rpm2cpio "$old_rpm" | cpio -idm 2>/dev/null)
old="$T/old/usr/bin/gantry"
[ -x "$old" ] || { echo "$old_rpm does not contain /usr/bin/gantry" >&2; exit 2; }
[ "$("$old" --version)" != "$("$new" --version)" ] || { echo "$new is $("$new" --version), the same as the package: build this tree first (cargo build)" >&2; exit 2; }
web="$repo/web/dist"; [ -f "$web/index.html" ] || web="$T/web"
cat > "$T/gantry.toml" <<CONF
[server]
bind = "127.0.0.1:$port"
public_url = "http://127.0.0.1:$port"
web_root = "$web"
manual_root = "$web"
[database]
url = "$db_url"
[security]
secret_key = "$("$new" config generate-key)"
allow_insecure_outbound = true
[logging]
format = "text"
level = "warn"
[storage.blob]
path = "$T/blobs"
[ai.embeddings]
provider = "hash"
[addons]
dir = "$T/addons"
CONF

start() { # BINARY LOG
  "$1" --config "$T/gantry.toml" serve > "$2" 2>&1 &
  server=$!
  for _ in $(seq 1 100); do curl -fs "$base/readyz" >/dev/null && return 0; kill -0 "$server" 2>/dev/null || break; sleep 0.2; done
  cat "$2"; echo "the server did not start" >&2; exit 1
}
stop() { kill "$server"; wait "$server" 2>/dev/null || true; server=""; }

echo "==> $("$old" --version): a new installation, filled through its API"
psql "$admin_url" -q -c "CREATE DATABASE $db"
"$old" --config "$T/gantry.toml" migrate > "$T/old-migrate.log" 2>&1
printf 'correct horse battery staple' | "$old" --config "$T/gantry.toml" admin create --email admin@example.org --name "Morgan Reyes" > /dev/null
start "$old" "$T/old-server.log"
python3 "$here/upgrade/seed-0.1.0.py" "$base" "$T/ids.json"
before="$("$old" --config "$T/gantry.toml" audit verify)"
stop
case "$before" in *'"intact":true'*) ;; *) echo "the audit log was not intact before the upgrade: $before" >&2; exit 1 ;; esac

echo "==> $("$new" --version): migrate, start, check"
"$new" --config "$T/gantry.toml" migrate > "$T/new-migrate.log" 2>&1 || { cat "$T/new-migrate.log"; echo "the migration failed" >&2; exit 1; }
"$new" --config "$T/gantry.toml" migrate > /dev/null 2>&1 || { echo "running the migration a second time failed" >&2; exit 1; }
start "$new" "$T/new-server.log"
python3 "$here/upgrade/check-0.2.0.py" "$base" "$T/ids.json"
after="$("$new" --config "$T/gantry.toml" audit verify)"
stop
case "$after" in *'"intact":true'*) echo "ok   the audit log is intact across the upgrade: $after" ;; *) echo "FAIL the audit log after the upgrade: $after"; exit 1 ;; esac
# The server logs at "warn" here, so anything it wrote is worth reading: a clean upgrade is quiet.
if [ -s "$T/new-server.log" ]; then echo "FAIL the upgraded server logged warnings or errors:"; sed 's/\x1b\[[0-9;]*m//g' "$T/new-server.log" | head -20; exit 1; fi
echo "ok   the upgraded server logged no warnings or errors"
echo "The upgrade from 0.1.0 works on this database."

#!/usr/bin/env bash
# Run the browser tests against a freshly seeded local server.
#   dev/e2e.sh                 Chromium at desktop, tablet and phone sizes
#   GANTRY_E2E_WEBKIT=1 dev/e2e.sh   also Safari's engine (needs: cd web && npx playwright install webkit)
#   dev/e2e.sh auth.spec       pass any Playwright arguments through
set -euo pipefail
cd "$(dirname "$0")/.."
LOG=$(mktemp)
(cd web && { [ -d node_modules ] || npm install --no-audit --no-fund; })
(cd web && npm run build)
dev/run-local.sh >"$LOG" 2>&1 &
SERVER=$!
trap 'kill $SERVER 2>/dev/null || true; pkill -f "gantry --config dev/gantry.dev.toml serve" 2>/dev/null || true' EXIT
for _ in $(seq 1 300); do
  grep -q "Gantry is running" "$LOG" && break
  kill -0 $SERVER 2>/dev/null || { cat "$LOG"; echo "the server did not start" >&2; exit 1; }
  sleep 1
done
cd web
npx playwright test "$@"

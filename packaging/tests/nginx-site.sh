#!/usr/bin/bash
# Tests packaging/files/nginx-site against a real nginx: writing a new site, refreshing the
# site of an older version after an upgrade, and leaving alone what must be left alone.
# Needs nginx and openssl. Changes nothing outside a temporary directory.
#
#   packaging/tests/nginx-site.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tool="$here/../files/nginx-site"
command -v nginx >/dev/null || { echo "SKIP: nginx is not installed"; exit 0; }
command -v openssl >/dev/null || { echo "SKIP: openssl is not installed"; exit 0; }
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
mkdir -p "$T/conf.d" "$T/logs"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 2 -subj "/CN=gantry.test" -keyout "$T/key.pem" -out "$T/cert.pem" 2>/dev/null

# nginx -t against a configuration that contains only the site under test.
cat > "$T/nginx.conf" <<CONF
pid $T/nginx.pid;
error_log $T/logs/error.log;
events {}
http {
    access_log off;
    client_body_temp_path $T/body; proxy_temp_path $T/proxy; fastcgi_temp_path $T/fcgi; uwsgi_temp_path $T/uwsgi; scgi_temp_path $T/scgi;
    include $T/conf.d/*.conf;
}
CONF
cat > "$T/nginx" <<WRAP
#!/usr/bin/bash
if [ "\${1:-}" = "-v" ]; then exec nginx -v; fi
exec nginx -t -c "$T/nginx.conf" -e "$T/logs/error.log"
WRAP
chmod +x "$T/nginx"
export GANTRY_NGINX_SITE="$T/conf.d/gantry.conf" GANTRY_NGINX_TEMPLATE="$here/../files/nginx-gantry.conf.in" GANTRY_NGINX_BIN="$T/nginx" GANTRY_NGINX_NO_RELOAD=1
site="$GANTRY_NGINX_SITE"
# The log paths in the template are the packaged ones; nginx -t must be able to open them here.
template="$T/template.conf.in"; old_template="$T/old.conf.in"
sed -e "s|/var/log/nginx/|$T/logs/|g" "$GANTRY_NGINX_TEMPLATE" > "$template"
sed -e "s|/var/log/nginx/|$T/logs/|g" "$here/fixtures/nginx-gantry-0.1.0.conf.in" > "$old_template"
export GANTRY_NGINX_TEMPLATE="$template"

n=0; fail() { echo "FAIL: $*"; exit 1; }
ok() { n=$((n + 1)); echo "ok $n - $*"; }
has() { grep -q -- "$1" "$site" || fail "$2: expected '$1' in the site"; }

# 1. A new site.
"$tool" render gantry.test "$T/cert.pem" "$T/key.pem" >/dev/null
head -1 "$site" | grep -q '^# Managed by gantry-setup' || fail "a new site is marked as managed"
has 'location = /api/v1/ws' "new site"; has 'proxy_set_header Upgrade' "new site"; has 'error_page 502 503 504 /unavailable.html' "new site"
has 'server_name gantry.test;' "new site"; has "ssl_certificate     $T/cert.pem;" "new site"
grep -q '@[A-Z_0-9]*@' "$site" && fail "placeholders are left in the site"
"$T/nginx" -t >/dev/null 2>&1 || { "$T/nginx" -t; fail "nginx rejects a new site"; }
ok "a new site is complete and nginx accepts it"

# 2. Nothing to do: silent, unchanged.
before="$(sha256sum < "$site")"
out="$("$tool" refresh)"
if [ -n "$out" ] || [ "$before" != "$(sha256sum < "$site")" ]; then fail "refreshing a current site changed it or spoke: $out"; fi
ok "refreshing a current site does nothing"

# 3. The site of 0.1.0, with a raised upload limit, after the package was upgraded.
rm -f "$site"
GANTRY_NGINX_TEMPLATE="$old_template" "$tool" render old.example.org "$T/cert.pem" "$T/key.pem" >/dev/null
sed -i 's/client_max_body_size 26m;/client_max_body_size 101m;/' "$site"
grep -q 'location = /api/v1/ws' "$site" && fail "the 0.1.0 fixture already has the WebSocket location"
out="$("$tool" refresh)"
echo "$out" | grep -q 'updated .* for this version' || fail "an upgrade says what it did: $out"
has 'location = /api/v1/ws' "upgraded site"; has 'error_page 502 503 504 /unavailable.html' "upgraded site"
has 'server_name old.example.org;' "upgraded site keeps the host name"; has "ssl_certificate_key $T/key.pem;" "upgraded site keeps the key"
has 'client_max_body_size 101m;' "upgraded site keeps the upload limit"
if [ -e "$site.before-upgrade" ] || [ -e "$site.new" ]; then fail "temporary files are left behind"; fi
"$T/nginx" -t >/dev/null 2>&1 || fail "nginx rejects the upgraded site"
[ -z "$("$tool" refresh)" ] || fail "a second refresh is not quiet"
ok "an older managed site is brought up to date, keeping host, certificate and upload limit"

# 4. A site the administrator took over is never changed; they are told what is missing.
rm -f "$site"
GANTRY_NGINX_TEMPLATE="$old_template" "$tool" render mine.example.org "$T/cert.pem" "$T/key.pem" >/dev/null
sed -i '1d' "$site"; before="$(sha256sum < "$site")"
out="$("$tool" refresh)"
[ "$before" = "$(sha256sum < "$site")" ] || fail "a hand-maintained site was changed"
echo "$out" | grep -q 'no WebSocket location' || fail "the administrator is told what is missing: $out"
out="$("$tool" render other.example.org "$T/cert.pem" "$T/key.pem")"
if [ "$before" != "$(sha256sum < "$site")" ] || ! echo "$out" | grep -q 'left unchanged'; then fail "render replaced a hand-maintained site"; fi
ok "a hand-maintained site is left alone and its owner is told what to add"

# 5. If nginx would refuse the result, the old site is put back.
rm -f "$site"
GANTRY_NGINX_TEMPLATE="$old_template" "$tool" render broken.example.org "$T/missing-cert.pem" "$T/key.pem" >/dev/null
before="$(sha256sum < "$site")"
out="$("$tool" refresh)"
[ "$before" = "$(sha256sum < "$site")" ] || fail "a refused site was left in place"
echo "$out" | grep -q 'left as it was' || fail "a refused refresh is reported: $out"
[ ! -e "$site.before-upgrade" ] || fail "the backup was left behind"
ok "a result nginx refuses is rolled back"

# 6. No site at all (nginx is someone else's business on this machine): nothing happens.
rm -f "$site"
if [ -n "$("$tool" refresh)" ] || [ -e "$site" ]; then fail "refresh created a site where there was none"; fi
ok "nothing happens where nginx was never configured"

# 7. Values that would break out of the configuration are refused.
"$tool" render 'evil.example.org; return 200' "$T/cert.pem" "$T/key.pem" >/dev/null 2>&1 && fail "a host name with a semicolon was accepted"
"$tool" render gantry.test "$T/cert.pem; include /etc/passwd" "$T/key.pem" >/dev/null 2>&1 && fail "a certificate path with a semicolon was accepted"
[ ! -e "$site" ] || fail "a refused value still wrote a site"
ok "host names and paths that are not plain are refused"

echo "All $n nginx-site checks passed."

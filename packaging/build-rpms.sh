#!/usr/bin/env bash
# Build every Gantry RPM from this source tree into dist/gantry-VERSION/{core,addons}.
#
#   packaging/build-rpms.sh              compile inside rpmbuild from a clean copy of the
#                                        sources (needs network access for crates and npm)
#   packaging/build-rpms.sh --prebuilt   build with cargo and npm here first, then only
#                                        package the results (faster while developing)
#   packaging/build-rpms.sh --offline    as the default, but with vendored dependencies:
#                                        expects ./vendor (cargo vendor) and a warm npm cache
#
# Build on the oldest Enterprise Linux release you support: binaries built on EL 9 run on
# EL 9 and 10. The Vagrant machine in this repository is set up for exactly this.
set -euo pipefail
cd "$(dirname "$0")/.."

MODE=source
for arg in "$@"; do
  case "$arg" in
    --prebuilt) MODE=prebuilt ;;
    --offline)  MODE=offline ;;
    -h|--help)  sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option $arg" >&2; exit 2 ;;
  esac
done

command -v rpmbuild >/dev/null || { echo "rpmbuild not found: dnf install rpm-build" >&2; exit 1; }
VERSION="$(sed -n 's/^Version:[[:space:]]*//p' packaging/gantry.spec)"
CARGO_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[ "$VERSION" = "$CARGO_VERSION" ] || { echo "version mismatch: gantry.spec has $VERSION, Cargo.toml has $CARGO_VERSION" >&2; exit 1; }

TOP="$PWD/build/rpm"
OUT="$PWD/dist/gantry-$VERSION"
rm -rf "$TOP" "$OUT"
mkdir -p "$TOP"/{SOURCES,SPECS,BUILD,RPMS,SRPMS} "$OUT"/{core,addons}

EXCLUDES=(--exclude=./build --exclude=./dist --exclude=./.git --exclude=./.vagrant --exclude='./web/node_modules'
          --exclude='./web/test-results' --exclude='./web/playwright-report' --exclude='./target/debug' --exclude='./target/release/build'
          --exclude='./target/release/deps' --exclude='./target/release/incremental' --exclude='./target/release/.fingerprint'
          --exclude='./target/release/examples' --exclude='./target/CACHEDIR.TAG' --exclude='./target/.rustc_info.json' --exclude='./target/tmp')
RPMARGS=()
if [ "$MODE" = prebuilt ]; then
  echo "==> building the tree with cargo and npm"
  # The date shown under About for each component.
  export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(date +%s)}"
  GANTRY_BUILD_DATE="$(date -u -d "@$SOURCE_DATE_EPOCH" +%Y-%m-%d)"
  export GANTRY_BUILD_DATE
  cargo build --release --locked
  (cd web && { [ -d node_modules ] || npm ci --no-audit --no-fund; } && npm run build && npm run manual)
  RPMARGS+=(--with prebuilt)
else
  EXCLUDES+=(--exclude=./target --exclude=./web/dist --exclude=./docs/manual/_site)
  [ "$MODE" = offline ] && RPMARGS+=(--with offline) || EXCLUDES+=(--exclude=./vendor)
fi

echo "==> source archive"
tar "${EXCLUDES[@]}" --transform "s,^\.,gantry-$VERSION," -czf "$TOP/SOURCES/gantry-$VERSION.tar.gz" .

# On a machine that is not Enterprise Linux (no systemd RPM macros) the packages can still be
# built to inspect their contents and scripts, but they are not for installing.
if [ -z "$(rpm --eval '%{?_unitdir}')" ]; then
  echo "==> this is not an Enterprise Linux build host: producing INSPECTION packages only" >&2
  RPMARGS+=(--nodeps --define "dist .inspect" --define "_unitdir /usr/lib/systemd/system" --define "_sysusersdir /usr/lib/sysusers.d"
            --define "_tmpfilesdir /usr/lib/tmpfiles.d" --define "_libexecdir /usr/libexec" --define "_sharedstatedir /var/lib"
            --define "systemd_requires %{nil}" --define "systemd_post() %{nil}" --define "systemd_preun() %{nil}"
            --define "systemd_postun_with_restart() %{nil}" --define "tmpfiles_create() %{nil}")
fi

echo "==> rpmbuild"
rpmbuild --define "_topdir $TOP" "${RPMARGS[@]}" -ba packaging/gantry.spec

find "$TOP/RPMS" -name '*.rpm' | while read -r rpm; do
  case "$(basename "$rpm")" in
    gantry-addon-*) cp "$rpm" "$OUT/addons/" ;;
    *)              cp "$rpm" "$OUT/core/" ;;
  esac
done
cp "$TOP"/SRPMS/*.src.rpm "$OUT/" 2>/dev/null || true
install -m 0755 packaging/install.sh "$OUT/install.sh"
(cd "$OUT" && sha256sum core/*.rpm addons/*.rpm > SHA256SUMS)

echo
echo "Packages are in $OUT"
(cd "$OUT" && ls -1 core addons)

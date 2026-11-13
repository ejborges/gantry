#!/usr/bin/bash
# Install or upgrade Gantry from the packages beside this script, on Red Hat Enterprise
# Linux 9 or 10 or an equivalent (Rocky Linux, AlmaLinux, Oracle Linux). Nothing needs to
# be installed beforehand; dependencies come from the distribution's repositories.
#
#   ./install.sh                       core packages
#   ./install.sh --with nasa,pgvector  core packages and the named add-ons
#   ./install.sh --with all            core packages and every add-on
#
# Add-ons: nasa  pgvector  valkey  opensearch  objectstore
# Afterward, on a new server:  gantry-setup --hostname NAME --admin-email YOU@EXAMPLE.ORG
set -euo pipefail

[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
HERE="$(cd "$(dirname "$0")" && pwd)"
ADDONS=""
while [ $# -gt 0 ]; do
  case "$1" in
    --with)    ADDONS="${2:?--with needs a list}"; shift 2 ;;
    -h|--help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done
[ "$ADDONS" = all ] && ADDONS="nasa,pgvector,valkey,opensearch,objectstore"

command -v dnf >/dev/null || { echo "dnf not found: this installer is for Enterprise Linux 9 and 10" >&2; exit 1; }
EL="$(rpm -E '%{?rhel}')"
case "$EL" in
  9)
    # The default PostgreSQL on Enterprise Linux 9 is too old; version 16 is a module stream.
    if ! rpm -q postgresql-server >/dev/null 2>&1; then
      echo "==> selecting PostgreSQL 16"
      dnf -y module reset postgresql >/dev/null
      dnf -y module enable postgresql:16
    elif [ "$(rpm -q --qf '%{VERSION}' postgresql-server | cut -d. -f1)" -lt 16 ]; then
      echo "PostgreSQL $(rpm -q --qf '%{VERSION}' postgresql-server) is installed here; Gantry needs 16 or newer." >&2
      echo "Upgrade it first (dnf module switch-to postgresql:16, then postgresql-setup --upgrade)." >&2
      exit 1
    fi
    ;;
  10) ;;
  *) echo "unsupported release (found '${EL:-unknown}'); Enterprise Linux 9 or 10 is required" >&2; exit 1 ;;
esac

PACKAGES=("$HERE"/core/*.rpm)
IFS=, read -ra WANTED <<< "$ADDONS"
for a in "${WANTED[@]}"; do
  [ -n "$a" ] || continue
  match=("$HERE"/addons/gantry-addon-"$a"-[0-9]*.rpm)
  [ -e "${match[0]}" ] || { echo "no add-on package named '$a' in $HERE/addons" >&2; exit 1; }
  PACKAGES+=("${match[0]}")
  # Valkey is part of Enterprise Linux 10; on 9 it comes from EPEL.
  if [ "$a" = valkey ] && [ "$EL" = 9 ] && ! rpm -q epel-release >/dev/null 2>&1; then
    echo "==> enabling EPEL for Valkey"
    dnf -y install epel-release
  fi
done

if [ -f "$HERE/SHA256SUMS" ]; then
  (cd "$HERE" && sha256sum --quiet -c SHA256SUMS) || { echo "a package does not match SHA256SUMS; refusing to install" >&2; exit 1; }
fi

echo "==> installing ${#PACKAGES[@]} packages"
dnf -y install "${PACKAGES[@]}"

if [ -s /etc/gantry/secret.key ]; then
  echo
  echo "Gantry was upgraded. Check it with:  gantry doctor"
else
  echo
  echo "Next:  gantry-setup --hostname $(hostname -f 2>/dev/null || hostname) --admin-email you@example.org"
fi

#!/usr/bin/env bash
# Prepare an Enterprise Linux 9 or 10 machine for building, testing and packaging Gantry.
# Run by `vagrant up`; safe to run again (`vagrant provision`), and usable by hand on any
# EL machine:  sudo dev/provision.sh [username]
set -euo pipefail

DEV_USER="${1:-${SUDO_USER:-vagrant}}"
EL="$(rpm -E '%{?rhel}')"
RUST_VERSION="${GANTRY_RUST_VERSION:-stable}"
say() { printf '\n==> %s\n' "$*"; }

[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
case "$EL" in 9|10) ;; *) echo "Enterprise Linux 9 or 10 is required (found '${EL:-unknown}')" >&2; exit 1 ;; esac

say "Repositories"
dnf -y install dnf-plugins-core
if [ "$EL" = 9 ]; then
  # Newer application streams than the EL 9 defaults.
  dnf -y module reset postgresql nodejs >/dev/null
  dnf -y module enable postgresql:16 nodejs:22
  dnf config-manager --set-enabled crb
else
  dnf config-manager --set-enabled crb
fi
dnf -y install epel-release

say "Build tools"
dnf -y install gcc gcc-c++ make git rsync tar gzip which jq openssl python3 \
  rpm-build rpmdevtools rpmlint systemd-rpm-macros \
  nodejs npm

say "Run-time dependencies, for running from source and for installing the built RPMs"
dnf -y install postgresql-server postgresql-contrib pgvector nginx valkey podman policycoreutils-python-utils

say "Libraries Chromium needs for the browser tests"
dnf -y install nss nspr atk at-spi2-atk cups-libs libdrm libxkbcommon libXcomposite libXdamage libXfixes libXrandr \
  mesa-libgbm pango cairo alsa-lib libX11 libxcb libXext dbus-libs

say "Rust ($RUST_VERSION) for $DEV_USER"
if ! runuser -l "$DEV_USER" -c 'command -v cargo' >/dev/null 2>&1; then
  runuser -l "$DEV_USER" -c "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain $RUST_VERSION --component clippy,rustfmt"
fi
runuser -l "$DEV_USER" -c 'rustc --version && cargo --version && node --version'

say "Sign-in password for $DEV_USER"
if [ -n "${GANTRY_VM_PASSWORD:-}" ]; then
  echo "$DEV_USER:$GANTRY_VM_PASSWORD" | chpasswd
  echo "    $DEV_USER can sign in to the desktop and the console with the configured password"
fi

if [ "${GANTRY_DESKTOP:-0}" = 1 ]; then
  say "Desktop: GNOME, Firefox and the VMware display tools (the window can be any size)"
  dnf -y group install "Server with GUI"
  dnf -y install open-vm-tools open-vm-tools-desktop firefox
  systemctl enable --now vmtoolsd
  systemctl set-default graphical.target
  # No first-sign-in wizard for the development account.
  home="$(getent passwd "$DEV_USER" | cut -d: -f6)"
  install -d -o "$DEV_USER" -g "$DEV_USER" "$home/.config"
  echo yes > "$home/.config/gnome-initial-setup-done"
  chown "$DEV_USER:" "$home/.config/gnome-initial-setup-done"
  # Start the sign-in screen now rather than at the next boot.
  systemctl isolate graphical.target --no-block || true
fi

# Inside this machine Gantry listens on every address, so that the ports Vagrant forwards
# from your computer reach it (they arrive on the machine's network address, not on
# loopback). The machine is reachable only from your computer, and the dev key is public:
# never do this on a shared network.
if [ -d /vagrant ]; then
  cat > /etc/profile.d/gantry-dev.sh <<'PROFILE'
# Set by dev/provision.sh for the Vagrant development machine.
export GANTRY_BIND=0.0.0.0:8600
export GANTRY_VITE_HOST=0.0.0.0
PROFILE
fi

if [ -d /vagrant/web ]; then
  say "JavaScript dependencies and the test browser"
  runuser -l "$DEV_USER" -c 'cd /vagrant/web && npm ci --no-audit --no-fund && npx playwright install chromium'
fi

cat <<MSG

Ready. Typical next steps, as $DEV_USER in /vagrant:

  dev/test-db.sh start            start a throwaway PostgreSQL for tests and local runs
  cargo test --workspace          unit, integration, security, external and regression tests
  (cd web && npm test)            front-end unit tests
  dev/run-local.sh                build, seed sample data and serve on port 8600
                                  (it prints the address to use from your computer)
  dev/e2e.sh                      browser tests
  packaging/build-rpms.sh         build the RPMs into dist/

See README.md for the full guide.
MSG
if [ "${GANTRY_DESKTOP:-0}" = 1 ]; then
  echo "Desktop: sign in as $DEV_USER in the VMware window; Firefox there reaches Gantry at http://localhost:8600."
fi

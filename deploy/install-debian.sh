#!/usr/bin/env bash
# Install the NetworkTester server on Ubuntu or Debian, without Docker.
#
#   sudo bash deploy/install-debian.sh              build and install
#   sudo bash deploy/install-debian.sh --uninstall  remove it again
#
# What it does:
#   1. installs build tools (apt) and Rust (rustup) if they are missing
#   2. builds the server from this repository
#   3. installs it as /usr/local/bin/nt-server
#   4. creates the service user "ntserver" and a systemd service
#   5. opens UDP 7001 and TCP 7002 in ufw, if ufw is active
set -euo pipefail

BIN=/usr/local/bin/nt-server
UNIT=/etc/systemd/system/nt-server.service
BUILD=/opt/nt-build            # Rust toolchain and build files live here, not in your home folder

if [ "$(id -u)" -ne 0 ]; then
    echo "Run this as root:  sudo bash $0 $*" >&2
    exit 1
fi

have_systemd() { [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; }

if [ "${1:-}" = "--uninstall" ]; then
    if have_systemd; then
        systemctl disable --now nt-server 2>/dev/null || true
        rm -f "$UNIT"
        systemctl daemon-reload
    fi
    rm -f "$BIN"
    id ntserver >/dev/null 2>&1 && userdel ntserver || true
    if command -v ufw >/dev/null 2>&1 && ufw status | grep -q "Status: active"; then
        ufw delete allow 7001/udp >/dev/null 2>&1 || true
        ufw delete allow 7002/tcp >/dev/null 2>&1 || true
    fi
    echo "Removed. The build folder $BUILD (about 1 GB with Rust) was kept. Delete it with: sudo rm -rf $BUILD"
    exit 0
fi

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
[ -f "$REPO/Cargo.toml" ] || { echo "Cannot find Cargo.toml in $REPO" >&2; exit 1; }

echo "==> Installing build tools"
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq build-essential curl ca-certificates pkg-config

export CARGO_HOME="$BUILD/cargo" RUSTUP_HOME="$BUILD/rustup" CARGO_TARGET_DIR="$BUILD/target"
export PATH="$CARGO_HOME/bin:$PATH"
if ! command -v cargo >/dev/null 2>&1; then
    echo "==> Installing Rust (rustup, minimal profile)"
    mkdir -p "$BUILD"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path
fi

echo "==> Building the server (a few minutes the first time)"
cargo build --release --locked -p server --manifest-path "$REPO/Cargo.toml"

echo "==> Installing $BIN"
install -m 0755 "$CARGO_TARGET_DIR/release/server" "$BIN"

id ntserver >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin ntserver

if command -v ufw >/dev/null 2>&1 && ufw status | grep -q "Status: active"; then
    echo "==> Opening ports in ufw"
    ufw allow 7001/udp
    ufw allow 7002/tcp
fi

if have_systemd; then
    echo "==> Starting the service"
    install -m 0644 "$REPO/deploy/nt-server.service" "$UNIT"
    systemctl daemon-reload
    systemctl enable --now nt-server
    sleep 1
    systemctl --no-pager --lines=0 status nt-server || true
else
    echo "systemd is not running here, so no service was created."
    echo "Start the server by hand with:  $BIN"
fi

echo
echo "Done. The server listens on UDP 7001 and TCP 7002."
echo "Server addresses: $(hostname -I 2>/dev/null || echo unknown)"
echo "If this machine is in a cloud, also open UDP 7001 and TCP 7002 in the provider's firewall."

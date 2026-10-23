#!/usr/bin/env bash
# GitHub's Azure mirror has stalled on large WebKit downloads for >9 minutes.
# Use only Ubuntu's signed repositories for this disposable runner, with bounds
# on both quiet connections and the complete operation. No host sources change.
set -euo pipefail
sources=$(mktemp)
trap 'rm -f "$sources"' EXIT
cat > "$sources" <<'SOURCES'
deb [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] https://archive.ubuntu.com/ubuntu noble main universe
deb [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] https://archive.ubuntu.com/ubuntu noble-updates main universe
deb [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] https://security.ubuntu.com/ubuntu noble-security main universe
SOURCES
options=(
  -o "Dir::Etc::sourcelist=$sources" -o 'Dir::Etc::sourceparts=-'
  -o Acquire::Retries=2 -o Acquire::http::Timeout=30
  -o Acquire::https::Timeout=30 -o Acquire::ForceIPv4=true
  -o APT::Update::Error-Mode=any -o Dpkg::Use-Pty=0
)
export DEBIAN_FRONTEND=noninteractive
echo 'Updating signed Ubuntu package indexes (limit: 2 minutes)'
timeout --signal=TERM --kill-after=15s 120s sudo -n -E apt-get "${options[@]}" update
echo 'Installing native build dependencies without recommended extras (limit: 6 minutes)'
timeout --signal=TERM --kill-after=15s 360s sudo -n -E apt-get "${options[@]}" install -y --no-install-recommends \
  pkg-config libssl-dev libgtk-3-dev libwebkit2gtk-4.1-dev \
  libayatana-appindicator3-dev librsvg2-dev patchelf

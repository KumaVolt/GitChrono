#!/usr/bin/env bash
# System libraries gpui needs to build and run on Debian/Ubuntu.
set -euo pipefail
apt-get update
apt-get install -y --no-install-recommends \
  pkg-config clang cmake \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-xcb-dev libxcb1-dev libxcb-xkb-dev \
  libfontconfig1-dev libfreetype-dev \
  libvulkan-dev libzstd-dev libssl-dev libdbus-1-dev

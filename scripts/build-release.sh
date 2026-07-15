#!/bin/bash
# Builds the release binary for GitHub Releases: SDL3 and all Rust deps
# statically linked, glibc and libmpv dynamic.
#
# Runs in a Debian 12 container, which pins the two runtime requirements as
# low as practical while still linking the current libmpv ABI (libmpv.so.2,
# mpv >= 0.35): the result runs on any x86_64 distro with glibc >= 2.36
# (Debian 12 / Ubuntu 23.04 / Fedora 37 or newer) and mpv installed.
#
# The container needs the dev headers of every SDL backend we want compiled
# in -- SDL dlopens the actual libraries at runtime, so backends missing on
# the build machine are silently absent from the binary forever.
#
# Requires docker or podman. Output: dist/tiny-media-center
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

engine=$(command -v podman || command -v docker)
mkdir -p dist

"$engine" run --rm \
    -v "$PWD:/src" \
    -v tiny-media-center-cargo:/root/.cargo \
    -w /src \
    debian:12 \
    bash -euo pipefail -c '
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq
        apt-get install -y -qq --no-install-recommends \
            build-essential cmake pkg-config curl ca-certificates \
            libmpv-dev \
            libx11-dev libxext-dev libxrandr-dev libxcursor-dev libxfixes-dev \
            libxi-dev libxss-dev libxtst-dev libxkbcommon-dev \
            libwayland-dev wayland-protocols libdecor-0-dev \
            libegl1-mesa-dev libgl1-mesa-dev libgles2-mesa-dev libdrm-dev libgbm-dev \
            libasound2-dev libpulse-dev libpipewire-0.3-dev libjack-jackd2-dev \
            libudev-dev libdbus-1-dev libibus-1.0-dev libusb-1.0-0-dev \
            > /dev/null
        [ -x "$HOME/.cargo/bin/cargo" ] ||
            curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
        export PATH="$HOME/.cargo/bin:$PATH"
        cargo build --release --target-dir /tmp/target
        install /tmp/target/release/tiny-media-center dist/
    '

echo
echo "== dist/tiny-media-center =="
echo "-- dynamic dependencies (expect libmpv + system basics, no libSDL3):"
objdump -p dist/tiny-media-center | grep NEEDED
echo "-- minimum glibc:"
objdump -T dist/tiny-media-center | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1

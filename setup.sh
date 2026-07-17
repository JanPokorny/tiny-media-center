#!/usr/bin/env bash
# One-shot dev setup for Fedora 44+: run as `./setup.sh` from the repo root
# (sudo is used internally for dnf). Installs the distro build dependencies
# (libmpv headers and SDL3's Linux build deps, since SDL3 is compiled from
# source by default), then installs mise and the pinned toolchain
# (`mise install`).
set -euo pipefail

command -v dnf >/dev/null || { echo "dnf not found; this script assumes Fedora" >&2; exit 1; }

repo_dir=$(cd "$(dirname "$0")" && pwd)
sudo=sudo
[ "$(id -u)" -eq 0 ] && sudo=

echo "== installing distro build dependencies (sudo dnf)"
# libmpv headers (by pkg-config capability, so the package name can't drift),
# cmake + the SDL3 feature deps from SDL's docs/README-linux.md Fedora list
# (SDL dlopens backends at runtime, so dev headers missing at build time mean
# the backend is silently absent from the binary forever), and gcc/curl for
# rustup/mise.
$sudo dnf install -y \
    gcc git-core make cmake pkgconf-pkg-config curl \
    'pkgconfig(mpv)' \
    alsa-lib-devel pulseaudio-libs-devel pipewire-devel \
    pipewire-jack-audio-connection-kit-devel \
    libX11-devel libXext-devel libXrandr-devel libXcursor-devel \
    libXfixes-devel libXi-devel libXScrnSaver-devel \
    dbus-devel ibus-devel systemd-devel \
    mesa-libGL-devel mesa-libGLES-devel mesa-libEGL-devel vulkan-devel \
    libxkbcommon-devel wayland-devel wayland-protocols-devel \
    libdrm-devel mesa-libgbm-devel libdecor-devel \
    libusb1-devel liburing-devel

mise=mise
if ! command -v mise >/dev/null; then
    if [ -x "$HOME/.local/bin/mise" ]; then
        mise="$HOME/.local/bin/mise"
    else
        echo "== installing mise (https://mise.run)"
        curl -fsSL https://mise.run | sh
        mise="$HOME/.local/bin/mise"
    fi
fi

echo "== mise install (pinned toolchain from mise.toml)"
"$mise" trust --quiet "$repo_dir/mise.toml"
(cd "$repo_dir" && "$mise" install)

echo
echo "done. If mise isn't activated in your shell yet, see"
echo "https://mise.jdx.dev/getting-started.html (e.g. for bash:"
echo "  echo 'eval \"\$(~/.local/bin/mise activate bash)\"' >> ~/.bashrc"
echo "), then build with: mise run build"

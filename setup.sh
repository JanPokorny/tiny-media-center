#!/usr/bin/env bash
# One-shot dev setup for Fedora 44+: run as `sudo ./setup.sh` from the repo
# root. Installs the distro build dependencies (libmpv headers and SDL3's
# Linux build deps, since SDL3 is compiled from source by default), then
# installs mise and the pinned toolchain (`mise install`) for the user who
# invoked sudo, not for root.
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "run as root: sudo ./setup.sh" >&2; exit 1; }
command -v dnf >/dev/null || { echo "dnf not found; this script assumes Fedora" >&2; exit 1; }

repo_dir=$(cd "$(dirname "$0")" && pwd)

echo "== installing distro build dependencies (dnf)"
# libmpv headers (by pkg-config capability, so the package name can't drift),
# cmake + the SDL3 feature deps from SDL's docs/README-linux.md Fedora list
# (SDL dlopens backends at runtime, so dev headers missing at build time mean
# the backend is silently absent from the binary forever), and gcc/curl for
# rustup/mise.
dnf install -y \
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

# The rest runs as the real user: mise installed into root's home would be
# useless for day-to-day builds.
target_user=${SUDO_USER:-root}
as_user() {
    if [ "$target_user" = root ]; then "$@"; else runuser -u "$target_user" -- "$@"; fi
}
user_home=$(getent passwd "$target_user" | cut -d: -f6)

if ! as_user bash -c 'command -v mise || [ -x "$HOME/.local/bin/mise" ]' >/dev/null; then
    echo "== installing mise for $target_user (https://mise.run)"
    as_user bash -c 'curl -fsSL https://mise.run | sh'
else
    echo "== mise already installed for $target_user"
fi
mise="$user_home/.local/bin/mise"
if as_user bash -c 'command -v mise' >/dev/null; then mise=mise; fi

echo "== mise install (pinned toolchain from mise.toml)"
as_user "$mise" trust --quiet "$repo_dir/mise.toml"
(cd "$repo_dir" && as_user "$mise" install)

echo
echo "done. If mise isn't activated in your shell yet, see"
echo "https://mise.jdx.dev/getting-started.html (e.g. for bash:"
echo "  echo 'eval \"\$(~/.local/bin/mise activate bash)\"' >> ~/.bashrc"
echo "), then build with: mise run build"

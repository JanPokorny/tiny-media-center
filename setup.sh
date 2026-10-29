#!/usr/bin/env bash
# One-shot dev setup for Fedora 44+: run as `./setup.sh` from the repo root
# (sudo is used internally for dnf). Installs the distro build and runtime
# dependencies (libmpv and SDL3 headers), then installs mise and the pinned
# toolchain (`mise install`).
set -euo pipefail

command -v dnf >/dev/null || { echo "dnf not found; this script assumes Fedora" >&2; exit 1; }

repo_dir=$(cd "$(dirname "$0")" && pwd)
sudo=sudo
[ "$(id -u)" -eq 0 ] && sudo=

echo "== installing distro build dependencies (sudo dnf)"
# libmpv and SDL3 headers (by pkg-config capability, so the package names
# can't drift) and gcc/curl for rustup/mise.
$sudo dnf install -y \
    gcc git-core make pkgconf-pkg-config curl \
    'pkgconfig(mpv)' 'pkgconfig(sdl3)'

echo "== installing runtime dependencies (sudo dnf)"
# mpv-libs (libmpv.so.2, the embedded player) already comes with the mpv
# headers above; pactl and bluetoothctl back the audio output menu. The rest
# is what session/tmc-session runs (X, compositor).
$sudo dnf install -y mpv-libs pulseaudio-utils bluez \
    xorg-x11-server-Xorg xorg-x11-xinit xrdb xprop picom
# dolphin-emu launches Wii games; it lives in RPM Fusion free, so skip it
# with a warning when that repo isn't enabled.
if ! $sudo dnf install -y dolphin-emu; then
    echo "warning: dolphin-emu is not available (it needs RPM Fusion free:" >&2
    echo "https://rpmfusion.org/Configuration); Wii game launching won't work" >&2
fi

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

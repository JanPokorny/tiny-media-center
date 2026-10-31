#!/usr/bin/env bash
# Sets up the TV box (and the dev toolchain) on Fedora 44+ from the repo
# checkout; rerunnable. Everything that configures the box lives in this repo.
#   ./setup.sh [system|user|all]
# system: packages, GNOME removal, session unit, updates; needs root (sudo
#         is called). user: the tv user's config, mise and the pinned
#         toolchain; run as tv (as root it switches to tv). Default: both.
# Never restarts sshd, the network, the user manager or tmc-session, never
# reboots: a replaced unit or conf applies at the next boot.
set -euo pipefail

repo_dir=$(cd "$(dirname "$0")" && pwd)
media=${MEDIA_DIR:-/media}

system() {
    command -v dnf >/dev/null || { echo "dnf not found; this script assumes Fedora" >&2; exit 1; }

    # Never remove or downgrade these (SSH/network/auth/package manager, what
    # Paseo and the session need): every dnf removal is simulated against it first.
    protected='^(openssh.*|firewalld.*|NetworkManager.*|tailscale|systemd.*|pam|pam-.*|authselect.*|polkit.*|sudo|dnf.*|libdnf.*|python3-libdnf.*|rpm|rpm-.*|ca-certificates|openssl.*|curl|libcurl.*|glibc.*|fprintd.*|libfprint|sssd.*|xorg-x11-.*|picom|bluez.*|pipewire.*|wireplumber.*|pulseaudio-utils|flatpak.*|xdg-desktop-portal|xdg-desktop-portal-gtk|steam|dolphin-emu|mpv-libs|SDL3.*|mesa.*|.*intel-media-driver|libva.*|ImageMagick.*|xprop|xrdb|jellyfin.*|wpa_supplicant|chrony|dbus.*|fish)$'
    preflight() {
        local out rows bad
        out=$(LC_ALL=C dnf "$@" --assumeno 2>&1) || true
        grep -q '^Transaction Summary' <<<"$out" || { echo "$out"; echo "ABORT: dnf could not resolve: dnf $*" >&2; exit 1; }
        rows=$(awk '/^[A-Z]/{sec=$0} /^ [^ ]/ && sec !~ /^Transaction Summary/{print sec "\t" $1}' <<<"$out")
        [[ -n $rows ]] || { echo "$out"; echo "ABORT: could not parse dnf transaction table" >&2; exit 1; }
        bad=$(awk -F'\t' '$1 ~ /^(Removing|Downgrading)/{print $2}' <<<"$rows" | grep -E "$protected" || true)
        [[ -z $bad ]] || { echo "$out"; echo "ABORT: transaction would remove/downgrade protected packages: $bad" >&2; exit 1; }
        echo "    preflight OK: dnf $*"
    }

    echo "== linger for tv (user manager, and Paseo with it, starts at boot)"
    loginctl enable-linger tv

    echo "== installing packages"
    # libmpv and SDL3 headers (by pkg-config capability, so the package names
    # can't drift), gcc/curl for mise; mpv-libs (libmpv.so.2, the embedded
    # player); pactl and bluetoothctl for the audio menu; X, compositor and
    # xauth/xinit/setpriv for session/start-x; wol and flatpak for the /media
    # launchers; dnf5 automatic updates.
    dnf install -y \
        gcc git-core make pkgconf-pkg-config curl \
        'pkgconfig(mpv)' 'pkgconfig(sdl3)' \
        mpv-libs pulseaudio-utils bluez \
        xorg-x11-server-Xorg xorg-x11-drv-libinput xorg-x11-xinit xorg-x11-xauth xrdb xprop picom \
        wol flatpak firefox dnf5-plugin-automatic
    # RPM Fusion: dolphin-emu launches Wii games (free), steam is the Steam
    # client (nonfree); skip with a warning when the repo isn't enabled.
    for pkg in dolphin-emu steam; do
        dnf install -y "$pkg" || echo "warning: $pkg is not available (it needs RPM Fusion, https://rpmfusion.org/Configuration)" >&2
    done
    # Fedora's autoremove would otherwise take pactl (pulled in as a dependency elsewhere).
    dnf mark user pulseaudio-utils

    echo "== flatpaks"
    flatpak remote-add --system --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
    flatpak install --system -y flathub com.valvesoftware.SteamLink rocks.shy.VacuumTube

    echo "== automatic updates (apply all, never reboot)"
    printf '[commands]\napply_updates = yes\nupgrade_type = default\nreboot = never\n' > /etc/dnf/automatic.conf
    systemctl enable --now dnf5-automatic.timer

    echo "== session unit (a copy: SELinux rejects units labelled user_home_t)"
    install -m 644 "$repo_dir/session/tmc-session.service" /etc/systemd/system/tmc-session.service
    restorecon -F /etc/systemd/system/tmc-session.service
    systemctl daemon-reload
    systemctl enable tmc-session.service
    systemctl set-default graphical.target
    systemctl cat gdm.service >/dev/null 2>&1 && systemctl disable gdm.service

    # Not removable on a Workstation without taking others along, so they stay:
    # gnome-keyring (NetworkManager-vpnc-gnome; its D-Bus activation is blocked
    # in the user part), colord (cups), geoclue2 (xdg-desktop-portal, firefox,
    # steam), accountsservice/malcontent-libs (flatpak), tecla (anaconda-gui).
    remove=(
        # GDM and the GNOME shell/session
        gdm gnome-shell gnome-shell-common gnome-session gnome-session-wayland-session gnome-settings-daemon
        mutter mutter-common gnome-keyring-pam evolution-data-server evolution-data-server-langpacks
        gnome-online-accounts gnome-online-accounts-libs gvfs-goa malcontent gnome-control-center
        gnome-control-center-filesystem xdg-desktop-portal-gnome switcheroo-control iio-sensor-proxy xdg-user-dirs-gtk
        gnome-desktop3 gnome-desktop4 gnome-bluetooth-libs gnome-autoar
        gnome-backgrounds desktop-backgrounds-gnome f42-backgrounds-gnome f43-backgrounds-gnome f44-backgrounds-gnome
        # GNOME apps and session extras
        baobab evince evince-djvu evince-previewer evince-thumbnailer gnome-disk-utility gnome-epub-thumbnailer
        gnome-system-monitor gnome-text-editor gnome-user-docs gnome-user-share loupe ptyxis yelp
        nautilus nautilus-extensions nautilus-python blueman-nautilus gnome-bluetooth gnome-initial-setup
        gnome-classic-session gnome-shell-extension-common gnome-shell-extension-apps-menu
        gnome-shell-extension-places-menu gnome-shell-extension-window-list gnome-shell-extension-launch-new-instance
        gnome-shell-extension-background-logo gnome-shell-extension-appindicator gnome-browser-connector
        gnome-menus gnome-app-list gnome-remote-desktop
        # Software center, indexer, Exchange backend, crash reporting, screen reader, input methods
        gnome-software gnome-software-fedora-langpacks PackageKit PackageKit-glib PackageKit-command-not-found
        PackageKit-gstreamer-plugin PackageKit-gtk3-module
        localsearch tinysparql evolution-ews-core evolution-ews-langpacks
        abrt abrt-libs abrt-cli abrt-tui abrt-dbus abrt-addon-ccpp abrt-addon-kerneloops abrt-addon-pstoreoops
        abrt-addon-vmcore abrt-addon-xorg abrt-plugin-bodhi python3-abrt python3-abrt-addon
        orca speech-dispatcher speech-dispatcher-espeak-ng speech-dispatcher-utils
        ibus ibus-anthy ibus-anthy-python ibus-chewing ibus-hangul ibus-m17n ibus-panel ibus-setup
        ibus-typing-booster python3-ibus
        # Other window managers and notification/Bluetooth UIs (tmc is all of them)
        openbox openbox-libs dunst blueman
    )
    mapfile -t todo < <(rpm -q --qf '%{NAME}\n' "${remove[@]}" | grep -v ' is not installed' || true)
    echo "== removing ${#todo[@]} of ${#remove[@]} listed packages still installed"
    if ((${#todo[@]})); then
        # Fedora Workstation protects gnome-shell; a leading empty element resets
        # the list, so every other protected.d entry (and dnf5) stays protected.
        dnf_protected="--setopt=protected_packages=,dnf5,$(grep -hvxE '#.*|[[:space:]]*|gnome-shell' /etc/dnf/protected.d/*.conf | paste -sd,)"
        preflight remove --noautoremove "$dnf_protected" "${todo[@]}"
        # --noautoremove: autoremove would also take pactl and much else.
        dnf remove -y --noautoremove "$dnf_protected" "${todo[@]}"
    fi

    echo
    echo "==================== SANITY ===================="
    failed=0
    check() { if eval "$2"; then echo "OK   $1"; else echo "!!!!! FAIL: $1 !!!!!"; failed=1; fi; }
    check "sshd -t" '/usr/sbin/sshd -t'
    check "sshd enabled" '[[ $(systemctl is-enabled sshd) == enabled ]]'
    check "sshd active" 'systemctl is-active -q sshd'
    check "port 22 listening" "ss -tlnH | grep -q ':22 '"
    check "linger tv" 'loginctl show-user tv -p Linger | grep -q =yes'
    check "pitchfork (Paseo) active" 'systemctl --user -M tv@ is-active -q pitchfork'
    check "tmc-session enabled" '[[ $(systemctl is-enabled tmc-session.service) == enabled ]]'
    check "default target graphical" '[[ $(systemctl get-default) == graphical.target ]]'
    check "gdm gone" '! rpm -q gdm >/dev/null'
    echo "================================================"
    ((failed)) && echo "!!!!! SOME CHECKS FAILED - do NOT reboot or log out of SSH before fixing them !!!!!" || echo "All checks passed. Reboot to apply the unit (not done automatically)."
    return "$failed"
}

user() {
    export XDG_RUNTIME_DIR=/run/user/$(id -u)
    cfg=$HOME/.config

    echo "== linking session files"
    mkdir -p "$cfg/wireplumber/wireplumber.conf.d" "$cfg/systemd/user"
    ln -sfn "$repo_dir"/session/wireplumber/*.conf "$cfg/wireplumber/wireplumber.conf.d/"
    for f in "$repo_dir"/session/media/*.sh; do ln -sfn "$f" "$media/"; done
    systemctl --user link "$repo_dir/session/steam-games.service" "$repo_dir/session/steam-games.path"
    systemctl --user enable --now steam-games.path

    echo "== blocking gnome-keyring's D-Bus activation (the package can't go, see system)"
    mkdir -p "$HOME/.local/share/dbus-1/services"
    for name in org.freedesktop.secrets org.freedesktop.impl.portal.Secret org.gnome.keyring; do
        printf '[D-BUS Service]\nName=%s\nExec=/bin/false\n' "$name" > "$HOME/.local/share/dbus-1/services/$name.service"
    done

    echo "== firefox: no client-side window buttons (tmc has no minimize/maximize)"
    for p in "$HOME"/.mozilla/firefox/*.default*; do
        grep -qs browser.tabs.inTitlebar "$p/user.js" || echo 'user_pref("browser.tabs.inTitlebar", 0);' >> "$p/user.js"
    done

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
}

case ${1:-all} in
    system) if [ "$(id -u)" -eq 0 ]; then system; else exec sudo "$0" system; fi ;;
    user) if [ "$(id -u)" -eq 0 ]; then exec runuser -u tv -- env HOME=/home/tv "$0" user; else user; fi ;;
    all) "$0" system && "$0" user ;;
    *) echo "usage: $0 [system|user|all]" >&2; exit 1 ;;
esac

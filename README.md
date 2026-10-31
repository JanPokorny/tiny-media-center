# tiny media center

A tiny fullscreen media center for keyboard/remote/gamepad use: browse a media
directory, play videos through embedded mpv (with per-video audio/subtitle
track memory and watch progress), launch Wii games through Dolphin, run shell
scripts. Written in Rust on SDL3 + femtovg + libmpv.

## run

On Fedora 43+, install from
[Copr](https://copr.fedorainfracloud.org/coprs/janpokorny/tiny-media-center/):

```sh
sudo dnf copr enable janpokorny/tiny-media-center
sudo dnf install tiny-media-center
```

On other distros:

1. install mpv 0.35+ from your distro (`pacman -S mpv` / `apt install libmpv2`)
2. download `tiny-media-center` from
   [Releases](https://github.com/JanPokorny/tiny-media-center/releases)
   (x86_64, needs glibc 2.36+ — Debian 12 / Ubuntu 23.04 / Fedora 37 or newer)

Then run it; it browses `/media`.

## the TV box

On the author's TV box tmc is the whole X11 desktop, with no display manager
or other window manager. At boot, the system unit
[session/tmc-session.service](session/tmc-session.service) opens a logind
session for `tv` on tty1 and runs [session/start-x](session/start-x), which
starts X and [session/tmc-session](session/tmc-session). That starts picom,
then restarts tmc (the window manager, which also shows the volume pop-up and
notifications, and is the Bluetooth pairing agent) from `/media` until X
goes away. systemd restarts the whole session if X dies.

### Setting up the box

Everything that configures the box lives in this repo and is applied by
[setup.sh](setup.sh) (rerunnable; `./setup.sh system` as root or via sudo,
`./setup.sh user` as `tv`, no argument for both): packages (installs what the
session needs, removes GNOME), the session unit, automatic dnf updates, Flatpaks,
and `tv`'s config symlinked from the repo (WirePlumber, the
[session/media](session/media) launchers into `/media`, the Steam library
watcher). It never reboots or restarts the session: reboot afterwards.

## configure

Optional, at `~/.config/tiny-media-center/config.toml`: OpenSubtitles
credentials for downloading missing subtitles (the API key comes from
opensubtitles.com profile > API consumers; the account is optional and raises
the daily download limit from 5 to 20).

```toml
[subtitles]
opensubtitles_api_key = ""
opensubtitles_username = ""
opensubtitles_password = ""
```

## build from source

`cargo build --release` — needs libmpv and SDL3 headers (Fedora:
`dnf install mpv-devel SDL3-devel`; `./setup.sh` installs everything on the box). It
links the system SDL3; `--features vendored-sdl` compiles SDL3 from source
and links it statically instead (needs cmake and SDL's
[Linux build dependencies](https://wiki.libsdl.org/SDL3/README-linux); this
is how the portable release binary is built).

With [mise](https://mise.jdx.dev), `mise install` provides the pinned Rust
toolchain and `mise tasks` lists the available tasks (`build-portable` and `release`).

## release

`mise run release` picks a patch/minor/major version bump, updates
Cargo.toml, commits, and pushes a `v*` tag. The tag runs the
[release workflow](.github/workflows/release.yml), which builds the
portable x86_64 binary (same recipe as the mise `build-portable` task),
attaches it to a GitHub Release, and triggers a rebuild of the
[Copr repository](https://copr.fedorainfracloud.org/coprs/janpokorny/tiny-media-center/)
from [tiny-media-center.spec](tiny-media-center.spec) (via
[.copr/Makefile](.copr/Makefile), which vendors the Rust dependencies at
SRPM time).

## credits

`subfont.ttf` = Kode Mono Regular from Google Fonts (SIL OFL 1.1)

## license

[GPL-2.0-or-later](LICENSE) (the embedded mpv player is GPL)

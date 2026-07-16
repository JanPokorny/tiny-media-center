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

Then run it in your media directory.

## configure

Optional, at `~/.config/tiny-media-center/config.toml`:

```toml
media_path = "/media"

[style]
font_size = 72
background_color = [0, 0, 0]
text_color = [1, 1, 1]
accent_color = [1, 0.8, 0]
dim_color = [0.5, 0.5, 0.5]
```

## build from source

`cargo build --release` — needs libmpv headers (`apt install libmpv-dev`,
on Arch plain `mpv` suffices) and SDL's
[Linux build dependencies](https://wiki.libsdl.org/SDL3/README-linux)
(SDL3 is compiled from source and statically linked by default; build with
`--no-default-features` to link a system SDL3 instead).

With [mise](https://mise.jdx.dev), `mise install` provides the pinned Rust
toolchain and `mise tasks` lists the available tasks (`build`, `run`,
`check`, `build-portable`, and `release`).

## release

`mise run release` picks a patch/minor/major version bump, updates
Cargo.toml, commits, and pushes a `v*` tag. The tag runs the
[release workflow](.github/workflows/release.yml), which builds the
portable x86_64 binary (same recipe as the mise `build-portable` task),
attaches it to a GitHub Release, and triggers a rebuild of the
[Copr repository](https://copr.fedorainfracloud.org/coprs/janpokorny/tiny-media-center/)
from [tiny-media-center.spec](tiny-media-center.spec) (via
[.copr/Makefile](.copr/Makefile), which vendors the Rust dependencies at
SRPM time), linking Fedora's system SDL3 instead of the vendored static
one.

## credits

`subfont.ttf` = Kode Mono Regular from Google Fonts (SIL OFL 1.1)

## license

[GPL-2.0-or-later](LICENSE) (the embedded mpv player is GPL)

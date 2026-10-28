# tiny media center

A tiny fullscreen media center for keyboard/remote/gamepad use: browse a media
directory, play videos through embedded mpv (with per-video audio/subtitle
track memory and watch progress), launch Wii games through Dolphin, run shell
scripts. Written in Rust on SDL3 + femtovg + libmpv.

## run

Run `tiny-media-center` in your media directory (needs libmpv at runtime).

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

The embedded mpv player reads `~/.config/tiny-media-center/mpv/mpv.conf`;
it is created with defaults on first run and your edits to it are kept.

## build

`cargo build --release` — needs libmpv headers and SDL's
[Linux build dependencies](https://wiki.libsdl.org/SDL3/README-linux)
(SDL3 is compiled from source and statically linked by default; build with
`--no-default-features` to link a system SDL3 instead). `mise install`
provides the pinned Rust toolchain.

## credits

`subfont.ttf` = Kode Mono Regular from Google Fonts (SIL OFL 1.1)

## license

[GPL-2.0-or-later](LICENSE) (the embedded mpv player is GPL)

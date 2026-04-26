# tiny media center

A minimal, fullscreen media browser and launcher for the living room. Browse a
directory of videos, Wii games, and scripts; play them with `mpv` (or
`dolphin-emu` / `bash`); and remember where you left off.

Built with [LÖVE](https://love2d.org/). Designed to be driven by a keyboard,
mouse, or gamepad.

## Features

- Directory-based browsing with a starfield background
- Plays videos through `mpv`, resuming from the last position
- Tracks audio and subtitle track selection per video
- Watched / in-progress indicators and a progress bar
- Auto-downloads subtitles via [`subliminal`](https://github.com/Diaoul/subliminal)
- Optional Wii game launching via [`dolphin-emu`](https://dolphin-emu.org/)
- Optional shell scripts as menu entries
- Configurable colors and font size

## Requirements

- [LÖVE 11.5](https://love2d.org/)
- [`mpv`](https://mpv.io/) on `PATH` (for video playback)
- [`subliminal`](https://github.com/Diaoul/subliminal) on `PATH` (optional, for subtitles)
- [`dolphin-emu`](https://dolphin-emu.org/) on `PATH` (optional, for `.rvz` Wii games)
- `bash` and standard POSIX tools (`find`)

## Run

```sh
git clone https://github.com/JanPokorny/tiny-media-center.git
cd tiny-media-center
love .
```

Or use the included `start` script.

## Configuration

Configuration lives at `$XDG_CONFIG_HOME/tiny-media-center/config.toml` (or
`~/.config/tiny-media-center/config.toml`). All keys are optional.

```toml
media_path = "/path/to/your/media"

[style]
background_color = [0.0, 0.0, 0.0]
text_color       = [1.0, 1.0, 1.0]
accent_color     = [1.0, 0.8, 0.0]
dim_color        = [0.5, 0.5, 0.5]
font_size        = 72
```

## Media library layout

Anything under `media_path` is browsable. File extensions decide how an entry
is treated:

| Extension                    | Type      | Action                         |
| ---------------------------- | --------- | ------------------------------ |
| `.mp4`, `.mkv`, `.avi`, `.mp3` | video     | Play with `mpv`                |
| `.rvz`                       | Wii game  | Launch with `dolphin-emu`      |
| `.sh`                        | script    | Run with `bash`                |

Hidden files and directories (anything starting with `.`) are skipped.

For each video, a `.tmc` sidecar file is written next to the media to remember
duration, last position, available tracks, and the chosen audio/subtitle IDs.
These are TOML and safe to delete; they will be rebuilt on next scan.

## Controls

| Action       | Keyboard      | Mouse        | Gamepad |
| ------------ | ------------- | ------------ | ------- |
| Move up      | Up            | Wheel up     | D-pad up |
| Move down    | Down          | Wheel down   | D-pad down |
| Select       | Enter         | Left click   | A |
| Back         | Esc           | Right click  | B |
| Restart app  | R             | —            | — |
| Jump to letter | any letter  | —            | — |

While a video is playing, `mpv` is launched with a custom `input.conf`
(see `attachments/mpv/input.conf`) — for example, the up/down arrows adjust
subtitle timing.

## Credits

- Bundled font: [Kode Mono Regular](https://fonts.google.com/specimen/Kode+Mono) (SIL OFL 1.1)
- Bundled TOML parser: [tinytoml](https://github.com/FourierTransformer/tinytoml) (MIT)
- Bundled mpv visualizer: [mfcc64/mpv-scripts](https://github.com/mfcc64/mpv-scripts) (Unlicense)

See [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md) for details.

## License

[MIT](LICENSE).

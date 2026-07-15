# tiny media center

## how to run

1. clone
2. install [Rust](https://rustup.rs/) and libmpv (`pacman -S mpv` / `apt install libmpv-dev`)
3. `cargo run --release`

Linking expects a system SDL3; on distros without an SDL3 package, build with
`cargo run --release --features vendored-sdl` to compile SDL3 from source instead.

## credits

`subfont.ttf` = Kode Mono Regular from Google Fonts

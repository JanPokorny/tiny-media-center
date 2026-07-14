# Rewrite exploration: moving off LÖVE/Lua

Goals, restated:

1. **Good performance on low-end devices** — snappy UI, fast startup, instant playback start.
2. **Concise and readable code** — the source stays tiny.
3. **Build mostly on built-in or library features** — no framework sprawl, no vendored 1500-line parsers.
4. **Possibly integrate mpv directly (libmpv)** instead of shelling out, if it buys perf or control.
5. Core tenet: **it's tiny** — source code *and* runtime footprint.

## What we have today (inventory)

Anything we rewrite must cover this. First-party code is ~950 lines of Lua:

| Part | Lines | Notes |
|---|---|---|
| `main.lua` | 589 | media tree, scan thread, menu wiring, mpv/dolphin launch, draw loop |
| `components/*.lua` | 271 | menu, menu item (squish anim), starfield background, loading screen |
| `config.lua`, `conf.lua` | 23 | TOML config with defaults |
| `vendor/tinytoml.lua` | 1540 | vendored TOML parser (dead weight in any rewrite) |
| `attachments/mpv/preflight.lua` | 22 | spawn-mpv-per-file to extract duration + track list to stdout |
| `attachments/mpv/runtime.lua` | 7 | reports playback position on stdout at exit |
| `attachments/mpv/visualiser.lua` | 367 | third-party audio visualizer (mfcc64), runs inside mpv |

Behaviours: recursive media scan (`find` via `io.popen`), extension→type map, `.tmc` TOML sidecars for position/duration/track metadata, subtitle fetch via `subliminal`, mpv spawned per playback with CLI args and stdout parsing, Dolphin via `systemd-run`, shell scripts via `bash`, input from keyboard/mouse/wheel/gamepad (including remote-control keys `appback`/`sleep`), Wayland fullscreen.

## Where the actual performance is

Honest framing first: LÖVE is LuaJIT on SDL2 — the menu itself is *not* slow, and a rewrite won't make a 60 fps menu 10× smoother. The real low-end wins are elsewhere:

- **Playback start latency.** Every play spawns a fresh `mpv` process: process fork, config parse, Lua script load, window creation, Wayland surface handoff between two apps. With libmpv in-process, the player core is initialized **once at startup** and `loadfile` is near-instant, rendering into the window we already own — no compositor handoff, no focus juggling, no black-screen gap.
- **Library scan.** Preflight today spawns one whole `mpv` process per new file. With libmpv the same instance loads each file paused, reads `duration`/`track-list` properties, moves on.
- **Runtime footprint.** LÖVE drags in LuaJIT, OpenAL, freetype, its own SDL init, ~50–80 MB RSS for what is a text menu. A compiled binary is ~1–3 MB, links only what it uses, and shares `libmpv` with the player. Startup goes from "load framework, JIT-warm scripts" to exec.
- **What a rewrite does *not* fix:** video decode/render perf is mpv's job either way (`hwdec`, `profile=fast`, `gpu-next` all still apply — the render API goes through the same libplacebo path, and hwdec GL interop keeps working).

## The libmpv question (matters more than the language)

The architecture decision is bigger than the language decision, so it comes first.

### Option I — keep spawning `mpv` (status quo model)

- ✅ Crash isolation: a decoder crash on a weird file kills mpv, not the media center.
- ✅ UI stack stays trivial (any 2D renderer works; no GL ownership).
- ❌ Startup latency per play; window/focus handoff on Wayland; control limited to CLI args + parsing stdout of our injected Lua scripts (`preflight.lua`, `runtime.lua` exist *only* to smuggle data out over stdout).
- Possible upgrade while keeping this model: talk JSON IPC (`--input-ipc-server`) to the spawned mpv instead of stdout scripts. Real control, still out-of-process. Middle ground worth knowing about, but keeps the spawn latency.

### Option II — libmpv render API, in-process (recommended)

mpv is designed for this: [`render.h`](https://github.com/mpv-player/mpv/blob/master/libmpv/render.h) lets us drive video rendering inside our own OpenGL context; the canonical example is ~200 lines of C ([mpv-examples/libmpv/sdl/main.c](https://github.com/mpv-player/mpv-examples/blob/master/libmpv/sdl/main.c)).

What it deletes / simplifies:

- `preflight.lua` + one-mpv-process-per-file scan → read `duration` and `track-list` properties directly (typed node tree, no string munging like `track_audio_3="eng/5.1ch"`).
- `runtime.lua` + stdout parsing → observe `time-pos` continuously; position is always current (also survives crashes/power cuts, which the current on-exit write does not).
- `input.conf` + mpv keybinding config → we own the event loop; seek/pause/sub-delay are one `command()` call each, same code path as menu navigation. Remote-control keys handled uniformly.
- CLI arg assembly + quoting (`--aid=`, `--sid=`, `--start=`) → `set_property` calls.
- Seamless UX: menu → video → menu in one fullscreen window, no compositor handoff, and we can draw our own overlay (progress bar, end-time) *over* the video instead of relying on mpv OSD if we ever want to.

What it costs:

- **We must own a GL context.** The render API wants to render into our OpenGL context, which rules out lazy 2D via `SDL_Renderer` on top. (mpv's `--wid` embedding doesn't exist on Wayland, and the software render backend would forfeit the GPU path — both dead ends for us.) So the menu gets drawn by ~300 lines of our own GL: one texture-atlas font (all glyphs the menu needs, rasterized once), one quad shader. That's the price of admission, paid once — and it natively gives us the fade (per-vertex alpha), the squish (x-scale), and the additive starfield that LÖVE's shader/blend features provide today.
- **Crash isolation is lost** — a libmpv crash takes the UI down. Mitigation: the `start` script already `exec`s us; run under a systemd user unit with `Restart=always`, and since position is observed continuously, a crash loses nothing. Acceptable for a single-user HTPC.
- **Threading rules** to respect: render on the GL thread, events on wakeup; don't call the core from the render callback. Well-documented, and the bindings encode most of it.
- `visualiser.lua` still works: libmpv loads Lua scripts (`load-scripts=yes`, `scripts=...`), per [mpv#5336](https://github.com/mpv-player/mpv/issues/5336) — script support is the full player core. (Its key binding uses mpv input, which we'd forward or drop; the visualization itself is `lavfi-complex`, which we could also just set as a property ourselves and delete the last 367 vendored lines.)

## Language / stack candidates

### A. Rust + SDL2 + libmpv2 — recommended

- **Bindings:** [`libmpv2`](https://crates.io/crates/libmpv2) (maintained fork of `libmpv-rs`, requires mpv ≥ 0.35) wraps client + render API, including the OpenGL `RenderContext`; [`rust-sdl2`](https://github.com/Rust-SDL2/rust-sdl2) covers window, GL context, keyboard, mouse, wheel, and game controllers (SDL's controller DB — same one LÖVE uses today, so gamepad behaviour carries over; `SDLK_AC_BACK`/`SDLK_SLEEP` cover the remote keys).
- **Batteries:** `toml` + `serde` derive replaces all 1540 lines of tinytoml with `#[derive(Serialize, Deserialize)]`; `std::fs` walks the media tree (no `find` subprocess, no `walkdir` dep needed at this size); `std::process` + read-child-stdout covers `subliminal`, Dolphin (`systemd-run` trick stays — it dodges inherited-process-state problems regardless of host language), and shell scripts.
- **Fit to goals:** compile-time types where the Lua version needs `---@class` annotations and runtime discipline; `Result`/`?` for the many "file might not exist" paths; single static binary ~2 MB linking only `libSDL2` and `libmpv` (both already on the box — mpv is required, LÖVE already pulled in SDL2).
- **Cost:** Rust toolchain for development; compile times (irrelevant at this size); GL layer is hand-written (fontdue or `ab_glyph` for rasterizing the existing `subfont.ttf` into an atlas).

### B. C++17 + SDL2 + libmpv

- The mpv API is C; the [official SDL example](https://github.com/mpv-player/mpv-examples/tree/master/libmpv) is directly usable. [`toml++`](https://marzer.github.io/tomlplusplus/) is a good header-only parser; `std::filesystem` walks the tree; `stb_truetype` for the atlas.
- Equally tiny binary, equally capable. Loses to Rust on: no package manager (deps are vendored headers or system packages + CMake), no serde (hand-written TOML⇄struct mapping), memory/UB safety for the string-heavy scanning/metadata code, and generally more ceremony per line — worse on the "concise and readable" goal. Choose this only if Rust toolchain aversion is strong.

### C. Rust + macroquad, keep spawning mpv

- [macroquad](https://macroquad.rs/) is the closest thing Rust has to LÖVE: immediate-mode 2D, text, shaders, tiny deps, near-line-for-line port of the current components. But it owns its GL context internally, so the libmpv render API doesn't fit — this stack locks in Option I (keep spawning mpv, or upgrade to JSON IPC). Gamepad needs a plugin crate ([`gamepads`](https://github.com/fornwall/gamepads)/gilrs) since macroquad has no native support.
- Right choice only if we decide *against* libmpv integration; then it's the fastest, most faithful port.

### D. Dismissed

- **JS/Electron** — ruled out by the prompt, and rightly: ~200 MB RSS floor.
- **Zig** — best-in-class C interop for libmpv, but pre-1.0 language churn and thin ecosystem (no TOML/serde equivalents of the same maturity) work against goals 2–3.
- **Go** — GC + cgo overhead on every libmpv/GL call, no good story for owning a GL render loop.
- **Stay on LÖVE, just adopt mpv JSON IPC** — the honest null option; fixes control but not the process-spawn latency, framework footprint, or type safety, and keeps tinytoml.

## Comparison

| | Rust+SDL2+libmpv2 | C+++SDL2+libmpv | Rust+macroquad+spawn | LÖVE (today) |
|---|---|---|---|---|
| Playback start | instant (warm core) | instant | process spawn | process spawn |
| Scan (new files) | in-process | in-process | 1 mpv proc/file | 1 mpv proc/file |
| Runtime deps | SDL2, mpv | SDL2, mpv | mpv (+X11/GL) | LÖVE stack, mpv |
| Binary / footprint | ~2 MB, ~25 MB RSS | ~1 MB, ~25 MB RSS | ~3 MB | ~50–80 MB RSS |
| Type/memory safety | ✅ / ✅ | static types, manual memory | ✅ / ✅ | annotations only |
| TOML | serde derive | toml++, manual mapping | serde derive | 1540 vendored lines |
| Crash isolation | ✗ (systemd restart) | ✗ (systemd restart) | ✅ | ✅ |
| Est. first-party LOC | ~1200–1400 | ~1500–1800 | ~900–1000 | ~950 (+1570 vendored/glue) |
| Hand-rolled parts | GL text layer (~300) | GL text layer (~300) | none | none |

Estimated Rust module breakdown: scanner ~120, metadata/sidecars ~80 (serde), menu/UI components ~350, GL + font atlas ~300, mpv module (init, render, events, properties) ~250, state/nav/main ~200. Slightly more code than today's Lua — but it deletes the 1540-line vendored parser, both stdout-smuggling mpv scripts, and the fragile CLI-quoting/parsing layer, while adding the entire player-control capability.

## Recommendation

**Rust + SDL2 + libmpv (render API), Option II architecture.** It is the only combination that hits all four goals at once: the perf wins come from the architecture (warm player core), the conciseness from serde/std, the "library features" from three mature deps (`sdl2`, `libmpv2`, `toml`), and the footprint from being a single small binary over libs the target machine already has.

Sketch of the core loop, to gauge the flavor (untested):

```rust
// init once
let mut mpv = Mpv::with_initializer(|i| {
    i.set_property("config-dir", cfg_dir)?;      // mpv.conf: hwdec, profile=fast...
    i.set_property("load-scripts", true)?;       // visualiser.lua keeps working
    i.set_property("vo", "libmpv")
})?;
let mut render_ctx = RenderContext::new(mpv.ctx, gl_get_proc_address)?;
mpv.observe_property("time-pos", Format::Double, 0)?;

// per frame
match app.mode {
    Mode::Menu => { draw_stars(&gl); draw_menu(&gl, &menu); }
    Mode::Playing => render_ctx.render::<GL>(0, w, h, true)?,  // mpv draws the frame
}

// play = two calls, not a quoted shell string
mpv.command("loadfile", &[&path])?;
mpv.set_property("start", meta.position)?;
```

### Migration plan

1. **Skeleton** — SDL2 window + GL context + font atlas + quad drawing; port menu/starfield/loading components. (The visual layer is the only genuinely new code; everything after is porting.)
2. **Scanner + metadata** — `std::fs` walk, serde TOML sidecars, config file. Delete tinytoml.
3. **libmpv playback** — render API integration, property observation, in-app keybindings (seek/pause/sub-delay/quit-to-menu). Delete `preflight.lua`, `runtime.lua`, `input.conf`, the arg-quoting code.
4. **Launchers** — Dolphin/`systemd-run` and shell scripts via `std::process` (mechanical port, keep the comment about why systemd-run).
5. **Parity checks on the target box** — Wayland fullscreen (`SDL_VIDEODRIVER=wayland`), gamepad/remote keys, hwdec active (`hwdec-current`), visualiser on mp3s, then swap the `start` script from `love` to the binary.

Fallback if step 3 turns hostile (render API friction on the target's GL/Wayland stack): the same Rust codebase degrades gracefully to spawning mpv with JSON IPC — everything except the mpv module survives unchanged. That's the risk-bounding property that makes starting this rewrite safe.

## Sources

- [libmpv2 crate](https://crates.io/crates/libmpv2) · [docs](https://docs.rs/libmpv2/latest/libmpv2/) · [repo](https://github.com/kohsine/libmpv2-rs)
- [mpv-examples: libmpv + SDL render API](https://github.com/mpv-player/mpv-examples/tree/master/libmpv) · [sdl/main.c](https://github.com/mpv-player/mpv-examples/blob/master/libmpv/sdl/main.c)
- [libmpv Lua script support discussion (mpv#5336)](https://github.com/mpv-player/mpv/issues/5336)
- [macroquad](https://macroquad.rs/) · [gamepads crate](https://github.com/fornwall/gamepads) · [gilrs](https://docs.rs/gilrs/latest/gilrs/)

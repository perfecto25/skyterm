# skyterm

A Terminator-like terminal emulator targeting system administrators and developers. Light on resource usage, GPU-rendered glyph drawing, split panes and tabs, themable, configurable fonts/colors.

**Status: M1 done; M2 mostly done; M3 substantially done (splits + menu + selection — tabs still pending). M4/M5 not started.** This file is the canonical project context. Read it cold before starting any work.

## Decisions already made

These are settled. Do not re-litigate without the user's say-so.

1. **Language: Rust.** Initially scaffolded as Crystal but switched after surveying the ecosystem. Rust has every load-bearing piece prebuilt.
2. **UI stack: GTK4 chrome + custom OpenGL `GLArea` for the terminal grid.** GTK4 gives us window/tabs/splits/menus/IME/clipboard for free; we own the hot path (glyph rendering) on the GPU.
3. **Platforms for v1: Linux Wayland, Linux X11, macOS.** GTK4 handles all three.
4. **v1 feature scope (everything must land before calling it v1):**
   - Splits (horizontal + vertical) + tabs + keyboard shortcuts + right-click context menu
   - Themes + configurable font family/size + colors + cursor style + opacity + scrollback length
   - xterm-level emulation: truecolor, 256-color, mouse reporting, bracketed paste
   - URL detection (Ctrl-click to open), in-buffer search, sixel images, font ligatures
5. **VT parser: `vte` crate** (Alacritty's). Push-driven; we implement `vte::Perform` to mutate our grid.
6. **Composition over `subclass!`**: panes are plain structs wrapping `GLArea` rather than GObject subclasses. Simpler, no GObject macro plumbing, no impact on functionality.

## Current implementation state

What's working end-to-end today:

- **PTY ↔ grid ↔ GPU loop.** `portable-pty` spawns `$SHELL`; bytes flow through `vte` → `skyterm_core::Grid` → renderer. PTY reads happen on a background thread; the parsed bytes hand off via `async_channel` to `glib::spawn_future_local` on the GLib main loop. No tokio.
- **xterm emulation: truecolor + 256-color SGR; cursor save/restore; bracketed paste; DEC graphics charset; DEC private modes (alt screen, cursor visibility); DECSTBM scroll region + IL/DL** (this last one is critical — without it, htop and ncurses apps that pin a header row corrupt the display after a few refreshes).
- **Glyph atlas.** FreeType rasterizes a default codepoint set into a single R8 atlas at startup. Two-pass render: opaque bg quads, then alpha-blended glyph quads. No HarfBuzz shaping yet (so no ligatures); each codepoint maps directly to one atlas slot.
- **Splits + focus.** `GtkPaned` tree, each pane owns its own PTY + grid + parser + renderer + atlas. Click-to-focus with a CSS-styled focus border. New pane after split inherits focus.
- **Chord shortcut.** Ctrl+A (tmux-style prefix) + arrow splits in that direction. Ctrl+A twice sends literal `0x01`. 2-second chord timeout.
- **Per-pane font zoom.** Ctrl+`+` / Ctrl+`-` / Ctrl+`0` and Ctrl+wheel re-rasterize the atlas at a new size on the focused pane only. Range 6–72 px.
- **Drag-to-select + copy.** `GestureDrag` on `BUTTON_PRIMARY` handles both focus-on-press (via inherited `begin` signal) and drag-to-select. Highlight rendered as a bg-color override in the renderer's first pass. Copy grabs the trimmed text.
- **Right-click menu.** `PopoverMenu` with three sections: splits, clipboard (Copy / Paste / Select All), and a destructive section with a custom red Close-pane button. Menu hides Close when only one pane exists. Fira Code font on menu items.
- **Snap-to-bottom on type.** Typing or pasting while scrolled up snaps the view back to the live screen before bytes reach the PTY.

Roughly 30 grid/parser unit tests live in `skyterm-core`.

## Stack — actual deps in use

| Concern | Crate | Version | Notes |
| --- | --- | --- | --- |
| GUI chrome | `gtk4` (gtk4-rs) | 0.11 (feat `v4_12`) | Targets GTK ≥ 4.12 — `compute_point` etc. |
| GL function loading | `glow` + `epoxy` + `libloading` | 0.13 / 0.1 / 0.8 | Wraps the `gdk::GLContext` from `GLArea`. |
| VT/xterm parser | `vte` | workspace | Alacritty's. We own the grid. |
| Font rasterization | `freetype-rs` | 0.36 | No `harfrust` yet — ligatures deferred. |
| PTY spawning | `portable-pty` | 0.9 | WezTerm's. Linux + macOS + Windows. |
| Channels | `async-channel` | 2 | Cross-thread PTY-read → GLib main loop hop. |
| Config | `serde` + `toml` | workspace | Wired through `skyterm-core/src/config.rs` only — no actual config file loaded yet. |
| Async / event loop | `glib::MainContext` | built-in | **No tokio.** |

Not yet added (planned for later milestones): `harfrust` (M2 ligatures), `icy_sixel` (M5 sixel), `linkify` (M5 URLs).

## Workspace layout

Reality differs from the original plan — splits/clipboard/popover all live in `app.rs` rather than separate modules. Refactor when it actually starts hurting; not yet.

```
skyterm/
├── Cargo.toml                 # workspace
├── skyterm-core/              # headless: parser + grid + theme model
│   ├── src/
│   │   ├── grid.rs            # cells, scrollback, scroll region, attrs
│   │   ├── parser.rs          # wraps `vte`, drives grid mutations
│   │   ├── theme.rs           # palette, fg/bg, cursor style
│   │   ├── config.rs          # serde structs (no loader wired in yet)
│   │   └── lib.rs
│   └── tests/                 # parser + grid tests, no GUI
├── skyterm-gui/               # GTK + GL rendering + input
│   ├── src/
│   │   ├── main.rs
│   │   ├── app.rs             # window, panes, splits, menu, selection, focus, chord — large
│   │   ├── renderer.rs        # glow: atlas upload, bg+glyph passes, Selection
│   │   ├── font.rs            # freetype atlas build
│   │   ├── input.rs           # key encoding for the PTY
│   │   └── pty.rs             # portable-pty + reader thread + async_channel
│   └── resources/             # (empty; reserved for themes/.ui)
└── docs/                      # config schema, keybindings, theme guide
```

`skyterm-core` has zero GTK/GL dependencies. `skyterm-gui` depends on `skyterm-core`.

## Implementation phases inside v1

1. **M1 — Hello PTY.** ✅ Done.
2. **M2 — Real text rendering.** ⚠️ Mostly done. Truecolor + 256-color, cursor styles (underline-only for now), bracketed paste, DEC charset, scroll region, IL/DL all working. **Still missing:** HarfBuzz shaping → ligatures, mouse reporting, italic/underline SGR attrs, vttest sweep.
3. **M3 — Splits, tabs, shortcuts, menu.** 🔄 Substantially done. Have: splits (Ctrl+A chord), right-click menu (splits + Copy/Paste/Select All + red Close), drag-select + copy, click-to-focus, per-pane font zoom. **Still missing:** tabs (GtkNotebook), pre-bound shortcut keymap config.
4. **M4 — Config & themes.** ❌ Not started. `config.rs` exists but is unloaded; everything is hardcoded.
5. **M5 — URLs, search, sixel.** ❌ Not started.

Shipping = M5 complete = v1.

## Hard-won lessons (don't re-learn the hard way)

These are real bugs that bit us. Document changes to the relevant subsystems carefully — they're fragile.

### GL handle ID collision across reparenting

When a pane is split, GTK4 unparents and reparents its `GLArea`, which destroys and recreates the GL context. Each fresh context restarts ID counters at 1 — so the old `Renderer`'s texture/program/VAO IDs and the new `Renderer`'s IDs collide. If you let the old `Renderer` drop *after* the new one is created (e.g. in `connect_realize` where you swap them), `Renderer::drop` deletes the new renderer's resources by ID. Result: blank pane forever.

**Fix: drop the renderer in `connect_unrealize`** — while the *old* context is still current. See `wire_pane` in `app.rs`. Don't re-introduce a drop-on-realize pattern.

### Popover sizing is parent-bound

`GtkPopover` (including `PopoverMenu`) sizes itself within its **parent widget's allocation**, not the surface. Parenting the popover to a `GLArea` inside a small split pane produces a squashed menu with scrollbars even when the window has plenty of room.

**Fix: parent popovers to the toplevel `ApplicationWindow`** (stored in `WindowState.window`) and translate click coords via `compute_point`. Then walk the popover's descendants in `connect_show` and disable `GtkScrolledWindow` policies — `PopoverMenu`'s internal scroller has its own clipping default.

### Gesture conflict on the same button

Two `GestureClick`s / `GestureDrag`s on the same button on the same widget *conflict*. Whichever claims the event sequence first locks the others out; `GestureClick` tends to win on press, leaving `GestureDrag` to never see motion.

**Fix: one gesture per button.** For LMB, use a single `GestureDrag` and hook its inherited `begin` signal for focus-on-press; `drag_begin` / `drag_update` fire after the motion threshold for selection. See `wire_pane`'s left-button block.

### DECSTBM is load-bearing

ncurses programs (htop, less, vim status lines) set a scroll region with `CSI r` to pin headers / status bars in place. Without it, every `\n` at the bottom scrolls the *whole* screen, corrupting the layout within seconds. The `Grid` tracks `scroll_top`/`scroll_bottom` and every linefeed / scroll / IL / DL consults it. Scrollback only accepts lines when the region covers the full screen — partial-region scrolls (htop's process list) must not leak into history.

### Snap-to-bottom on user input

Typing or pasting while scrolled into history should jump the view back to live. Call `snap_to_bottom(pane)` *before* writing bytes to the PTY at every send site (key handler default path, chord pass-through, paste).

### Per-pane atlas, per-pane font size

Font zoom is a per-pane operation (intentional, per user request). Each `Pane` owns its own `font_size`, `cell_dims`, and atlas in its renderer. Don't share an atlas across panes; you'll need to rebuild on every zoom anyway, and per-pane independence is the feature.

## Verification

- **Headless tests in `skyterm-core/tests/`** + inline `#[cfg(test)]` modules in `grid.rs` and `parser.rs`. ~30 tests covering scroll region, IL/DL, SGR, cursor save/restore, alt screen, scrollback, DEC graphics.
- **End-to-end per milestone**: see "Done when" in each M.
- **CI** (when set up): `cargo test -p skyterm-core` on Linux; `cargo build -p skyterm-gui` on Linux + macOS.

## Platform caveats

- **macOS**: GTK4 via Homebrew (`brew install gtk4`). Quartz backend only. Expect quirks around CSD and fractional scaling.
- **Linux Wayland vs X11**: GTK4 handles both transparently. `set_allowed_apis(gdk::GLAPI::GL)` is required to force desktop GL on Wayland (Mesa would otherwise hand us a GLES context that breaks `#version 330` shaders).
- **OpenGL**: GL 3.3 core. `gl_area.set_required_version(3, 3)`.

## What NOT to do

- **Don't add `tokio`.** GLib `MainContext` is the executor; PTY I/O hops over `async_channel` from a reader thread. Two reactors = pure overhead.
- **Don't reach for `cosmic-text`** — wrong tool for a monospace cell grid.
- **Don't use `harfbuzz_rs` (the 2021 crate)** when you add shaping — use `harfrust`.
- **Don't FFI to `libvte`** — we use the `vte` *crate* (Williams parser only).
- **Don't ship "v1" without all five milestones.**
- **Don't add per-frame allocations.** Renderer scratch buffers (`bg_scratch`/`fg_scratch`) are reused. Same for any future hot loops.
- **Don't drop a `Renderer` while a different GL context is current** (see the lessons section).

## Reference reading

- Alacritty's `vte` crate docs for the `Perform` trait shape.
- WezTerm's `term/` crate for grid + scrollback patterns.
- The `gtk4-rs` book's `GLArea` chapter; the `glarea` example in the gtk4-rs examples repo for the `glow` loader pattern.
- Paul Williams' VT state machine spec at vt100.net/emu/dec_ansi_parser — background; the `vte` crate implements it for us.

## Full plan file

A more verbose version of the original plan lives at `/home/mreider/.claude/plans/this-is-a-terminal-swirling-globe.md`. This CLAUDE.md is the canonical project context.

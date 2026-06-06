# skyterm

A Terminator-like terminal emulator targeting system administrators and developers. Light on resource usage, GPU-rendered glyph drawing, split panes and tabs, themable, configurable fonts/colors.

**Status: M1 done; M2 mostly done (ligatures + italic/underline SGR still missing); M3 done (splits + tabs + menu + selection + shortcuts); M4 substantially done (config load/save + Settings + About + theme swap); M5 not started.** This file is the canonical project context. Read it cold before starting any work.

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

- **PTY ↔ grid ↔ GPU loop.** `portable-pty` spawns `$SHELL` (TERM=`xterm-256color`); bytes flow through `vte` → `skyterm_core::Grid` → renderer. PTY reads happen on a background thread; the parsed bytes hand off via `async_channel` to `glib::spawn_future_local` on the GLib main loop. No tokio.
- **xterm emulation: truecolor + 256-color SGR; cursor save/restore; bracketed paste; DEC graphics charset; DEC private modes (alt screen, cursor visibility); DECSTBM scroll region + IL/DL** (this last one is critical — without it, htop and ncurses apps that pin a header row corrupt the display after a few refreshes).
- **Mouse reporting.** `?1000` (normal), `?1002` (button-motion), `?1003` (any-motion), and `?1006` (SGR encoding) tracked on the grid. The scroll wheel and left-button press/drag/release are forwarded to the PTY as mouse reports when an app enables reporting (so htop tabs/rows, vim, etc. are clickable). **Shift overrides** forwarding to do local text selection. Right-click stays the context menu (not forwarded). Wire formats: SGR `CSI < b ; col ; row M/m`, X10 `CSI M Cb Cx Cy`.
- **Application cursor keys + F-keys.** DECCKM (`?1`) is tracked; arrows/Home/End transmit SS3 (`ESC O x`) when set, CSI otherwise — ncurses apps won't match them otherwise. F1–F4 send SS3 (`ESC O P…S`), F5–F12 send `CSI …~`, matching `xterm-256color` terminfo (htop's F3 search, etc.).
- **Line reflow on resize.** Already-printed text rewraps when the window/pane resizes (VTE-style). See the hard-won-lessons entry; resize is debounced so a drag fires one clean grid+PTY resize on settle.
- **Glyph atlas.** FreeType rasterizes a default codepoint set into a single R8 atlas at startup. Two-pass render: opaque bg quads, then alpha-blended glyph quads. No HarfBuzz shaping yet (so no ligatures); each codepoint maps directly to one atlas slot.
- **Tabs.** `GtkNotebook` strip; each tab (`struct Tab`) owns a `GtkPaned` split tree of panes. `pane.new-tab` action (from the right-click menu — no keyboard accel yet); `current_tab` / `tab_of_pane` resolve the active set.
- **Splits + focus.** `GtkPaned` tree per tab, each pane owns its own PTY + grid + parser + renderer + atlas. Click-to-focus with a CSS-styled focus border. New pane after split inherits focus.
- **Chord shortcut.** Ctrl+A (tmux-style prefix) + arrow splits in that direction. Ctrl+A twice sends literal `0x01`. 2-second chord timeout.
- **Per-pane font zoom.** Ctrl+`+` / Ctrl+`-` / Ctrl+`0` and Ctrl+wheel re-rasterize the atlas at a new size on the focused pane only. Range 6–72 px.
- **Drag-to-select + copy.** `GestureDrag` on `BUTTON_PRIMARY` handles focus-on-press (via inherited `begin` signal), drag-to-select, and — when mouse reporting is active — mouse forwarding. Highlight rendered as a bg-color override in the renderer's first pass. Copy grabs the trimmed text.
- **Right-click menu.** `PopoverMenu` sections: splits, clipboard (Copy / Paste / Select All), Settings + About, and a destructive section with a custom red Close-pane button. Menu hides Close when only one pane exists.
- **Settings + About windows.** Settings: font family/size, built-in theme, scrollback length, cursor-blink toggle — applied live to all panes and persisted. About: name, `CARGO_PKG_VERSION`, and a `LinkButton` to the repo.
- **Config persistence.** `skyterm-core::config::Config` (TOML at `$XDG_CONFIG_HOME/skyterm/config.toml`) is loaded at startup and saved from Settings: `font_path`, `font_size`, `theme_name`, `scrollback_lines`, `cursor_blink`, `click_word_select`, `copy_on_select`. All fields optional so old configs keep loading.
- **Double-click word / triple-click line selection.** Multi-click on the left button (counted manually in the `GestureDrag` `begin` handler via event time + cell, since a second gesture would conflict): double = word (`is_word_char` run), triple = whole logical line (expands across soft-wrapped rows via `grid.visible_row_wrapped`). Gated by `click_word_select`. Copy joins soft-wrapped rows without a `\n`.
- **Copy-on-select.** When `copy_on_select` is on, the `GestureDrag` `connect_end` handler copies the current selection (word, line, or drag) to the clipboard on button release. Default off.
- **Snap-to-bottom on type.** Typing or pasting while scrolled up snaps the view back to the live screen before bytes reach the PTY.

~39 grid/parser unit tests live in `skyterm-core` (reflow, mouse modes, DECCKM, scroll region, IL/DL, SGR, alt screen, scrollback, DEC graphics).

## Stack — actual deps in use

| Concern | Crate | Version | Notes |
| --- | --- | --- | --- |
| GUI chrome | `gtk4` (gtk4-rs) | 0.11 (feat `v4_12`) | Targets GTK ≥ 4.12 — `compute_point` etc. |
| GL function loading | `glow` + `epoxy` + `libloading` | 0.13 / 0.1 / 0.8 | Wraps the `gdk::GLContext` from `GLArea`. |
| VT/xterm parser | `vte` | workspace | Alacritty's. We own the grid. |
| Font rasterization | `freetype-rs` | 0.36 | No `harfrust` yet — ligatures deferred. |
| PTY spawning | `portable-pty` | 0.9 | WezTerm's. Linux + macOS + Windows. |
| Channels | `async-channel` | 2 | Cross-thread PTY-read → GLib main loop hop. |
| Config | `serde` + `toml` | workspace | Loaded at startup + saved from Settings (`$XDG_CONFIG_HOME/skyterm/config.toml`). |
| Async / event loop | `glib::MainContext` | built-in | **No tokio.** |

Not yet added (planned for later milestones): `harfrust` (M2 ligatures), `icy_sixel` (M5 sixel), `linkify` (M5 URLs).

## Workspace layout

Reality differs from the original plan — splits/clipboard/popover all live in `app.rs` rather than separate modules. Refactor when it actually starts hurting; not yet.

```
skyterm/
├── Cargo.toml                 # workspace
├── package-rpm.sh             # build release + cargo-generate-rpm  → .rpm
├── package-deb.sh             # build release + cargo-deb           → .deb
├── package-macos.sh           # .app bundle + dylibbundler + create-dmg (run on macOS)
├── skyterm-core/              # headless: parser + grid + theme model
│   ├── src/
│   │   ├── grid.rs            # cells, scrollback, scroll region, Row{wrapped}+reflow, mouse/DECCKM flags
│   │   ├── parser.rs          # wraps `vte`, drives grid mutations
│   │   ├── theme.rs           # palette, fg/bg, cursor style
│   │   ├── config.rs          # serde Config: load/save TOML
│   │   └── lib.rs
│   └── tests/                 # parser + grid tests, no GUI
├── skyterm-gui/               # GTK + GL rendering + input
│   ├── src/
│   │   ├── main.rs
│   │   ├── app.rs             # window, tabs, panes, splits, menu, settings, about, selection, focus, chord, reflow — large
│   │   ├── renderer.rs        # glow: atlas upload, bg+glyph passes, Selection
│   │   ├── font.rs            # freetype atlas build
│   │   ├── input.rs           # key encoding for the PTY (arrows/F-keys/DECCKM)
│   │   └── pty.rs             # portable-pty + reader thread + async_channel
│   └── resources/             # skyterm.svg icon, skyterm.desktop, JetBrainsMono-Regular.ttf (embedded fallback font)
└── docs/                      # config schema, keybindings, theme guide
```

Packaging metadata for `cargo-generate-rpm` and `cargo-deb` lives in `[package.metadata.*]` in `skyterm-gui/Cargo.toml`. Asset paths there are relative to `skyterm-gui/`, so the binary is `../target/release/skyterm`. The RPM/deb both install the binary, the SVG icon, and the `.desktop` launcher.

`skyterm-core` has zero GTK/GL dependencies. `skyterm-gui` depends on `skyterm-core`.

## Implementation phases inside v1

1. **M1 — Hello PTY.** ✅ Done.
2. **M2 — Real text rendering.** ⚠️ Mostly done. Truecolor + 256-color, cursor styles (underline-only for now), bracketed paste, DEC charset, scroll region, IL/DL, mouse reporting (?1000/1002/1003 + ?1006 SGR; wheel + button clicks/drags forwarded — Shift overrides to local selection), application cursor keys (DECCKM ?1) all working. **Still missing:** HarfBuzz shaping → ligatures, italic/underline SGR attrs, vttest sweep.
3. **M3 — Splits, tabs, shortcuts, menu.** ✅ Done. Splits (Ctrl+A chord), tabs (GtkNotebook + new-tab menu action), right-click menu (splits + Copy/Paste/Select All + Settings/About + red Close), drag-select + copy, click-to-focus, per-pane font zoom. **Possible follow-up:** keyboard accelerators for tab/pane switching (currently only the chord-splits and font-zoom are bound).
4. **M4 — Config & themes.** 🔄 Substantially done. `Config` loads at startup and saves from Settings (font family/size, theme, scrollback, cursor blink); built-in themes + user themes (`load_user_themes`); live apply across panes. **Still missing:** opacity, cursor-style selection in UI, broader config surface.
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

### Reflow on resize (we DO reflow, like VTE)

skyterm rewraps already-printed lines on resize, matching gnome-terminal/Terminator (VTE). The earlier assumption that we'd be xterm-style "no reflow" was wrong for a Terminator-like terminal.

Mechanism: each grid row is a `Row { cells, wrapped }`. `put_char` sets `wrapped = true` on the row it leaves *only* when it auto-wraps at the right edge — an explicit newline reaches `linefeed` directly and leaves it `false`. That soft-vs-hard distinction is the only thing `Grid::resize` needs: it rejoins runs of `wrapped` rows (scrollback + screen) into logical lines, re-splits at the new width, and keeps the cursor pinned to its text position. `Row` derefs/indexes to its inner `Vec<Cell>`, so the flag rides along through every `remove`/`insert`/`push_back` and can't desync.

Don't reflow the alt screen — `resize` dispatches to `resize_simple` (pad/truncate) when `alt.is_some()`, because TUIs (vim, htop) repaint themselves on SIGWINCH and reflow would fight them.

**The GUI must debounce reflow — do NOT reflow per resize event.** Reflow moves the cursor, and `reflow_to_pixels` also calls `master.resize()` which sends SIGWINCH → the shell reprints its prompt with cursor-*relative* escapes. A window drag fires ~20 resize events; if each one reflows, a later reflow moves the cursor *between* the shell's SIGWINCH and its multi-read redraw response, so the reprint lands misaligned and the prompt visibly appends/duplicates. The fix (`schedule_reflow` in `app.rs`): `connect_resize` only updates the GL viewport immediately and arms a 60 ms one-shot timer (cancelling any pending one). During an active drag the timer keeps resetting and never fires, so there's **one** grid+PTY resize on settle and exactly one clean shell redraw. An earlier per-frame reflow in `connect_render` plus an always-`queue_render` blink timer caused exactly this corruption — they were removed; don't bring them back. Reflow math uses **logical** pixels (`area.width()/height()`), not the signal's device pixels, or HiDPI computes the wrong column count. The split-divider drag (`notify::position`) routes through the same `schedule_reflow`.

### Input encoding: DECCKM and terminfo-matched keys

ncurses apps (htop, vim, less) call `keypad()`, which sets DECCKM (`?1h`) and then matches incoming key bytes against `xterm-256color` terminfo. Send the wrong form and the key is silently ignored. So `encode_key` takes the grid's `app_cursor_keys` flag and emits SS3 (`ESC O x`) for arrows/Home/End in app mode, CSI otherwise. F-keys are fixed encodings: F1–F4 = `ESC O P/Q/R/S`, F5–F12 = `CSI 15~/17~…24~` (htop's F3 search = `ESC O R`). When adding more keys, check `infocmp xterm-256color` rather than guessing.

Mouse reports: SGR (`?1006`) is `CSI < b ; col ; row M` (press/motion) or `m` (release); legacy X10 is `CSI M Cb Cx Cy` with each byte offset by 32 — note the **`[`**: it's `ESC [ M`, not `ESC M` (that's reverse-index RI and was a real bug in the scroll encoder).

### GTK4 CSS `!important` loses — never use it (dark menu)

The right-click menu must render dark or the accelerator hints are invisible on a light system theme. The fix is `CSS_DARK_MENU` (selectors on `popover.skyterm-menu …`), registered via `style_context_add_provider_for_display` at **priority 10 000** (well above the theme's 200). The non-obvious trap that cost three failed attempts: **adding `!important` to those declarations makes them LOSE to the theme's non-important rules**, so the menu stays light. GTK4's `!important` cascade does the opposite of the CSS spec here. Plain declarations at a high provider priority win cleanly — that's the only thing that works. Don't reintroduce `!important`.

Proven empirically: dump the popover's live node tree + `widget.color()` while it's realized (a magenta no-`!important` rule applied; the same rule with `!important` reverted to the theme default). The popover node tree is `popover.background.menu.skyterm-menu › contents › scrolledwindow › viewport › stack › box… › modelbutton{box,label,accelerator}`; the class is added with `popover.add_css_class("skyterm-menu")` right after `PopoverMenu::from_model`.

## Verification

- **Headless tests in `skyterm-core/tests/`** + inline `#[cfg(test)]` modules in `grid.rs` and `parser.rs`. ~39 tests covering reflow (widen/narrow/scrollback-boundary/alt-screen), mouse modes + SGR toggle, DECCKM, scroll region, IL/DL, SGR, cursor save/restore, alt screen, scrollback, DEC graphics.
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

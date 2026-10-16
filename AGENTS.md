# AGENTS.md — rules for any agent working on skyterm

`CLAUDE.md` has the project context (architecture, decisions, hard-won lessons). **Read it first.** This file is the one non-negotiable process rule:

## Run the full test suite after every build

After **every** build (`cargo build`, `cargo build --release`, or any change you compile), run the tests before you move on or tell the user you're done:

```sh
./build.sh          # cargo build + the whole suite   (or)
./run-tests.sh      # the whole suite only
```

- Plain `cargo build` / `cargo check` do **not** run tests. Do not treat a successful compile as verification.
- `./run-tests.sh` runs `cargo test --workspace`, using a virtual display (`xvfb-run`) when installed. It includes the GUI rule tests in `skyterm-gui/src/app_tests.rs` (menu content / size / placement / colour, tabs, panes, focus dimming, highlight colour, close confirmation, About window, config).
- The suite must be **green**. If anything fails, fix the code (or, if the rule genuinely changed, update the test *and say so*) before continuing. Report failures with the output; never say "tests pass" unless you ran them in this session and saw them pass.
- If you can't run the GUI test (no display, `SKYTERM_SKIP_GUI_TESTS=1`, macOS), say that explicitly — a skipped GUI test is not a pass.
- Packaging (`package-*.sh`) already runs the suite first and refuses to package on failure; CI (`.github/workflows/test.yml`) runs it on every push/PR.

## Write or update tests with every change

- **New or changed behaviour needs a test in the same change.** Pure logic → a unit test (see `dim_scrim`, `close_confirmation`, `normalize_hex_color`, `rule(...)`-style CSS assertions in `app_tests.rs`). Anything that needs the real widgets → a function called from `gui_end_to_end`.
- **Fixing a bug? Add the regression test first**, watch it fail, then fix. (Prove it can fail: temporarily re-break the code and confirm the test goes red.)
- **Don't weaken or delete a test to make it pass**, and don't `#[ignore]` one without telling the user why.
- Core emulation changes (`skyterm-core`: grid, parser, config) get tests in that crate; they need no display.

## Writing GUI tests (gotchas)

- All GTK checks live in the single `#[test] gui_end_to_end` — GTK must stay on one thread. Add a function and call it from there; don't add a second windowed `#[test]`.
- The harness points `XDG_CONFIG_HOME` / `XDG_DATA_HOME` at a temp dir and disables the session bus. Keep it that way: tests must never read or write the user's real config or touch their running skyterm.
- GTK stores widget opacity in 8 bits (compare with a tolerance) and re-serialises CSS colours as `rgba(r,g,b,a)` (use `css_has_color`).
- Validate any new CSS: GTK4 silently ignores unsupported properties (e.g. `overflow`) and `!important` makes rules *lose*. `css_parses_cleanly` fails on parse errors; keep new stylesheets covered by it.

## Running the app by hand

skyterm is single-instance (`dev.skyterm.Skyterm`): launching a second copy hands the launch to the user's already-running window and exits — and any xdotool input you send then goes to **their** session. To run a separate instance, unset the session bus and use scratch dirs:

```sh
DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent XDG_CONFIG_HOME=/tmp/x/cfg XDG_DATA_HOME=/tmp/x/data target/debug/skyterm
```

Prefer a test over a manual check wherever the behaviour can be asserted.

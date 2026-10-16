#!/usr/bin/env bash
# Run the whole test suite: skyterm-core's headless tests plus the GUI rule
# tests in skyterm-gui (menu content/size/placement/colour, tabs, panes,
# dimming, highlight colour, close confirmation, About window, config).
#
# The GUI end-to-end test boots the real app, so it needs a display. When
# `xvfb-run` is installed we use a throwaway virtual display so no windows
# flash up on your desktop; otherwise it uses $DISPLAY, and with no display at
# all it skips itself (the pure CSS/menu-model/logic tests still run).
#
#   ./run-tests.sh                 # everything
#   ./run-tests.sh -- --nocapture  # extra args go to `cargo test`
#   SKYTERM_SKIP_GUI_TESTS=1 ./run-tests.sh   # skip the windowed test
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

if command -v xvfb-run &>/dev/null; then
    exec xvfb-run -a -s "-screen 0 1280x1024x24" cargo test --workspace "$@"
fi
exec cargo test --workspace "$@"

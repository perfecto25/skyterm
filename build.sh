#!/usr/bin/env bash
# Build, then run the full test suite. Plain `cargo build` has no post-build
# hook, so use this (or `./run-tests.sh`) to get "build + verify" in one step.
#
#   ./build.sh            # debug build + tests
#   ./build.sh --release  # extra args go to `cargo build`
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

cargo build "$@"
./run-tests.sh

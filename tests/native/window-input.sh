#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail
cd "$(dirname "$0")/../.."
temporary="$(mktemp -d "${TMPDIR:-/tmp}/trames-window-input.XXXXXX")"
trap 'rm -rf "$temporary"' EXIT
xcrun clang -fobjc-arc -fblocks -Wall -Wextra -Werror -framework AppKit -framework Foundation -framework CoreFoundation \
  tests/native/window-input.m crates/trames-mac/src/window-input.m crates/trames-mac/src/window-clipboard.m -o "$temporary/window-input-test"
"$temporary/window-input-test"

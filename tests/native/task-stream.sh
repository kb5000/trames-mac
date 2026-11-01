#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail
cd "$(dirname "$0")/../.."
temporary="$(mktemp -d "${TMPDIR:-/tmp}/trames-task-stream.XXXXXX")"
trap 'rm -rf "$temporary"' EXIT
xcrun clang -fobjc-arc -fblocks -Wall -Wextra -Werror -framework Foundation -framework CoreFoundation \
  tests/native/task-stream.m crates/trames-mac/src/task-stream.m -o "$temporary/task-stream-test"
"$temporary/task-stream-test"

xcrun clang -fobjc-arc -fblocks -Wall -Wextra -Werror -framework AppKit -framework Foundation -framework CoreFoundation \
  tests/native/manager-progress.m crates/trames-mac/src/manager.m crates/trames-mac/src/task-stream.m -o "$temporary/manager-progress-test"
"$temporary/manager-progress-test"

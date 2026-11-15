#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail
cd "$(dirname "$0")/../.."
temporary="$(mktemp -d "${TMPDIR:-/tmp}/trames-task-stream.XXXXXX")"
trap 'rm -rf "$temporary"' EXIT
xcrun clang -fobjc-arc -fblocks -Wall -Wextra -Werror -framework Foundation -framework CoreFoundation \
  tests/native/task-stream.m crates/trames-mac/src/task-stream.m -o "$temporary/task-stream-test"
"$temporary/task-stream-test"

# Match the production static-archive linkage, including Objective-C categories.
# Native inputs: crates/trames-mac/src/manager.m crates/trames-mac/src/manager-ui.h
# crates/trames-mac/src/manager-actions.m crates/trames-mac/src/localization.h
for unit in manager manager-actions task-stream; do
  xcrun clang -c -fobjc-arc -fblocks -Wall -Wextra -Werror "crates/trames-mac/src/$unit.m" -o "$temporary/$unit.o"
done
xcrun ar -rcs "$temporary/libmanager.a" "$temporary/manager.o" "$temporary/manager-actions.o" "$temporary/task-stream.o"
xcrun clang -fobjc-arc -fblocks -Wall -Wextra -Werror -framework AppKit -framework Foundation -framework CoreFoundation \
  tests/native/manager-progress.m "$temporary/libmanager.a" -o "$temporary/manager-progress-test"
"$temporary/manager-progress-test"

# Exercise the actual bundle layout: Finder's entry differs from the CLI binary.
bundle="$temporary/ManagerFixture.app"
mkdir -p "$bundle/Contents/MacOS"
cp "$temporary/manager-progress-test" "$bundle/Contents/MacOS/trames-manager"
cp "$temporary/manager-progress-test" "$bundle/Contents/MacOS/trames-mac-run"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>org.trames.tests.manager-entry</string><key>CFBundleExecutable</key><string>trames-manager</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>
PLIST
"$bundle/Contents/MacOS/trames-manager"

# Same native input/clipboard implementation as the lazy Wayland window frontend.
bash tests/native/window-input.sh

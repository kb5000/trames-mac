#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail
cd "$(dirname "$0")/.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
cargo build -p trames-mac --release --locked
# A minimal agent bundle supplies AppKit identity without a startup window.
# The CLI re-execs this binary in place; stdin/stdout and the PID are retained.
bundle=target/release/Trames.app
mkdir -p "$bundle/Contents/MacOS"
cp target/release/trames-mac-run target/release/trames-mac-package.tmp
rm -f "$bundle/Contents/MacOS/trames-mac-run"
mv target/release/trames-mac-package.tmp "$bundle/Contents/MacOS/trames-mac-run"
python3 - <<'PLIST'
import pathlib,plistlib,re
path=pathlib.Path('target/release/Trames.app/Contents/Info.plist')
path.write_bytes(plistlib.dumps(dict(CFBundleIdentifier='org.trames.Trames.mac',CFBundleName='Trames',CFBundleDisplayName='Trames',CFBundleExecutable='trames-mac-run',CFBundlePackageType='APPL',CFBundleShortVersionString=re.search(r'version = "([^"]+)"',pathlib.Path('Cargo.toml').read_text())[1],CFBundleVersion='1',LSUIElement=True,NSHighResolutionCapable=True)))
PLIST
identity="${TRAMES_SIGN_IDENTITY:--}"
if [[ "$identity" == "-" ]]; then
  codesign --force --sign - --entitlements apps/macos/Trames.entitlements "$bundle"
else
  codesign --force --sign "$identity" --options runtime --timestamp --entitlements apps/macos/Trames.entitlements "$bundle"
fi
codesign --verify --strict "$bundle"
rm -f target/release/trames-mac-run
ln "$bundle/Contents/MacOS/trames-mac-run" target/release/trames-mac-run

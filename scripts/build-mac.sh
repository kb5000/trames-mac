#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail
cd "$(dirname "$0")/.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
export TRAMES_BUILD_CHANNEL="${TRAMES_BUILD_CHANNEL:-dev}"
case "$TRAMES_BUILD_CHANNEL" in
  stable) bundle_name="Trames.app" ;;
  dev) bundle_name="Trames Dev.app" ;;
  *) echo 'TRAMES_BUILD_CHANNEL must be stable or dev' >&2; exit 1 ;;
esac
cargo build -p trames-mac --release --locked
# A minimal agent bundle supplies AppKit identity without a startup window.
# The CLI re-execs this binary in place; stdin/stdout and the PID are retained.
bundle="target/release/$bundle_name"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp apps/macos/Resources/Trames.icns "$bundle/Contents/Resources/Trames.icns"
cp -R apps/macos/Resources/en.lproj apps/macos/Resources/zh-Hans.lproj "$bundle/Contents/Resources/"
cp target/release/trames-mac-run target/release/trames-mac-package.tmp
rm -f "$bundle/Contents/MacOS/trames-mac-run"
mv target/release/trames-mac-package.tmp "$bundle/Contents/MacOS/trames-mac-run"
python3 - <<'PLIST'
import pathlib,plistlib,re
import os
channel=os.environ['TRAMES_BUILD_CHANNEL']
name='Trames' if channel=='stable' else 'Trames Dev'
path=pathlib.Path('target/release')/(name+'.app')/'Contents/Info.plist'
version=re.search(r'version = "([^"]+)"',pathlib.Path('Cargo.toml').read_text())[1]
path.write_bytes(plistlib.dumps(dict(CFBundleIdentifier='org.trames.Trames.mac'+('' if channel=='stable' else '.dev'),CFBundleName=name,CFBundleDisplayName=name,TramesChannel=channel,CFBundleExecutable='trames-mac-run',CFBundlePackageType='APPL',CFBundleIconFile='Trames.icns',CFBundleShortVersionString=version,CFBundleVersion=version,CFBundleDevelopmentRegion='en',CFBundleLocalizations=['en','zh-Hans'],LSUIElement=True,NSHighResolutionCapable=True,NSMicrophoneUsageDescription='Allow Linux applications in Trames to use your microphone when you enable audio input.')))
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

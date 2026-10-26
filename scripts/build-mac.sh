#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail
cd "$(dirname "$0")/.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
cargo build -p trames-mac --release --locked
codesign --force --sign - --entitlements apps/macos/Trames.entitlements target/release/trames-mac-run
codesign --verify --strict target/release/trames-mac-run

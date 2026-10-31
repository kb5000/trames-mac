#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Per-user install; environments live outside the application and survive upgrades.
set -euo pipefail
source_dir="$(cd "$(dirname "$0")" && pwd)"
source_app="${1:-$source_dir/Trames.app}"
app_dir="${TRAMES_APPLICATIONS_DIR:-$HOME/Applications}"
bin_dir="${TRAMES_BIN_DIR:-$HOME/.local/bin}"
/usr/bin/codesign --verify --strict "$source_app"
mkdir -p "$app_dir" "$bin_dir"
stage="$app_dir/.Trames-install-$$.app"
dest="$app_dir/Trames.app"
previous="$app_dir/.Trames-previous-$$.app"
rollback() {
  rm -rf "$stage"
  if [[ -d "$previous" && ! -e "$dest" ]]; then mv "$previous" "$dest"; fi
}
trap rollback EXIT
# Refuse CLI conflicts before replacing any installed application.
if [[ -e "$bin_dir/trames" || -L "$bin_dir/trames" ]]; then
  [[ -L "$bin_dir/trames" && "$(readlink "$bin_dir/trames")" == "$dest/Contents/MacOS/trames-mac-run" ]] || { echo "CLI path already exists: $bin_dir/trames. Choose a different TRAMES_BIN_DIR." >&2; exit 1; }
fi
/usr/bin/ditto "$source_app" "$stage"
/usr/bin/codesign --verify --strict "$stage"
if [[ -e "$dest" ]]; then
  identifier=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$dest/Contents/Info.plist")
  [[ "$identifier" == "org.trames.Trames.mac" ]] || { echo 'Destination is not a Trames installation.' >&2; exit 1; }
  mv "$dest" "$previous"
fi
mv "$stage" "$dest"
if [[ ! -L "$bin_dir/trames" ]]; then
  ln -s "$dest/Contents/MacOS/trames-mac-run" "$bin_dir/trames"
fi
rm -rf "$previous"
printf 'Installed %s\nCLI: %s/trames\nOpen Trames.app for environment management.\n' "$dest" "$bin_dir"

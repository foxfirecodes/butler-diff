#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
profile="${1:-debug}"
case "$profile" in
    debug) cargo build --locked ;;
    release) cargo build --locked --release ;;
    *) printf '%s\n' 'Usage: bundle-macos.sh [debug|release]' >&2; exit 1 ;;
esac
bundle="target/$profile/Butler Diff.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "target/$profile/butler-diff" "$bundle/Contents/MacOS/butler-diff.next"
mv -f "$bundle/Contents/MacOS/butler-diff.next" "$bundle/Contents/MacOS/butler-diff"
cp macos/Info.plist "$bundle/Contents/Info.plist"
cp macos/AppIcon.icns "$bundle/Contents/Resources/AppIcon.icns"
touch "$bundle"
printf 'built %s\n' "$PWD/$bundle"

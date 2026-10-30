#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build --locked
bundle="target/debug/Butler Diff.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp target/debug/butler-diff "$bundle/Contents/MacOS/butler-diff.next"
mv -f "$bundle/Contents/MacOS/butler-diff.next" "$bundle/Contents/MacOS/butler-diff"
cp macos/Info.plist "$bundle/Contents/Info.plist"
cp macos/AppIcon.icns "$bundle/Contents/Resources/AppIcon.icns"
touch "$bundle"
printf 'built %s\n' "$PWD/$bundle"

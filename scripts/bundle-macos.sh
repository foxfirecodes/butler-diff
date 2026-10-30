#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
profile="${1:-debug}"
variant="${2:-normal}"
case "$profile" in
    debug) cargo build --locked ;;
    release) cargo build --locked --release ;;
    *) printf '%s\n' 'Usage: bundle-macos.sh [debug|release] [normal|dev]' >&2; exit 1 ;;
esac
case "$variant" in
    normal) app_name="Butler Diff" ;;
    dev) app_name="Butler Diff Dev" ;;
    *) printf '%s\n' 'Expected normal or dev app variant.' >&2; exit 1 ;;
esac
bundle="target/$profile/$app_name.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "target/$profile/butler-diff" "$bundle/Contents/MacOS/butler-diff.next"
mv -f "$bundle/Contents/MacOS/butler-diff.next" "$bundle/Contents/MacOS/butler-diff"
cp macos/Info.plist "$bundle/Contents/Info.plist"
cp macos/AppIcon.icns "$bundle/Contents/Resources/AppIcon.icns"
if [ "$variant" = dev ]; then
    /usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier dev.foxfire.butler-diff.dev' "$bundle/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c 'Set :CFBundleName Butler Diff Dev' "$bundle/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c 'Set :CFBundleDisplayName Butler Diff Dev' "$bundle/Contents/Info.plist"
    touch "$bundle/Contents/Resources/Development"
else
    rm -f "$bundle/Contents/Resources/Development"
fi
touch "$bundle"
printf 'built %s\n' "$PWD/$bundle"

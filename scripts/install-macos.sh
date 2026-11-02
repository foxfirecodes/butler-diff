#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

source_app="target/release/Gander.app"
install_dir="${GANDER_INSTALL_DIR:-${BUTLER_DIFF_INSTALL_DIR:-/Applications}}"
target_app="$install_dir/Gander.app"
codesign --verify --deep --strict "$source_app"
mkdir -p "$install_dir"

# Stage and verify before replacing an existing installation; roll back if
# the final copy or signature check fails.
work="$(mktemp -d "$install_dir/.gander-install.XXXXXX")"
complete=false
cleanup() {
    if [ "$complete" = false ] && [ -d "$work/previous.app" ]; then
        rm -rf "$target_app"
        mv "$work/previous.app" "$target_app"
    fi
    rm -rf "$work"
}
trap cleanup EXIT HUP INT TERM

ditto "$source_app" "$work/Gander.app"
codesign --verify --deep --strict "$work/Gander.app"
if [ -e "$target_app" ]; then
    identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$target_app/Contents/Info.plist")"
    [ "$identifier" = dev.foxfire.gander ] || {
        printf '%s\n' "Refusing to replace an unrelated app: $target_app" >&2
        exit 1
    }
    mv "$target_app" "$work/previous.app"
fi
mv "$work/Gander.app" "$target_app"
codesign --verify --deep --strict "$target_app"
complete=true
printf 'installed %s\n' "$target_app"

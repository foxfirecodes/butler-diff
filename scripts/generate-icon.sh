#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

# Regenerate the macOS icon from the editable SVG. Requires librsvg's
# rsvg-convert; ordinary app builds use the generated .icns directly.
command -v rsvg-convert >/dev/null 2>&1 || {
    printf '%s\n' 'rsvg-convert is required to regenerate the app icon.' >&2
    exit 1
}

work="$(mktemp -d "${TMPDIR:-/tmp}/butler-diff-icon.XXXXXX")"
trap 'rm -rf "$work"' EXIT HUP INT TERM
iconset="$work/AppIcon.iconset"
mkdir -p "$iconset"

for size in 16 32 128 256 512; do
    retina=$((size * 2))
    rsvg-convert -w "$size" -h "$size" assets/icon.svg -o "$iconset/icon_${size}x${size}.png"
    rsvg-convert -w "$retina" -h "$retina" assets/icon.svg -o "$iconset/icon_${size}x${size}@2x.png"
done

iconutil -c icns "$iconset" -o macos/AppIcon.icns
printf 'generated %s\n' "$PWD/macos/AppIcon.icns"

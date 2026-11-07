#!/bin/sh
# Regenerates packaging/icons/* from fluxwire.svg (needs rsvg-convert; iconutil on macOS).
set -eu
cd "$(dirname "$0")/../packaging/icons"
command -v rsvg-convert >/dev/null || { echo "install librsvg (rsvg-convert)" >&2; exit 1; }
for s in 16 32 48 64 128 256 512 1024; do
  rsvg-convert -w "$s" -h "$s" fluxwire.svg -o "fluxwire-$s.png"
done
if command -v iconutil >/dev/null; then
  set_dir=fluxwire.iconset
  rm -rf "$set_dir"; mkdir "$set_dir"
  for s in 16 32 128 256 512; do
    cp "fluxwire-$s.png" "$set_dir/icon_${s}x${s}.png"
    d=$((s * 2)); cp "fluxwire-$d.png" "$set_dir/icon_${s}x${s}@2x.png" 2>/dev/null || rsvg-convert -w "$d" -h "$d" fluxwire.svg -o "$set_dir/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns "$set_dir" -o fluxwire.icns
  rm -rf "$set_dir"
fi
if command -v magick >/dev/null; then
  magick fluxwire-16.png fluxwire-32.png fluxwire-48.png fluxwire-256.png fluxwire.ico
fi

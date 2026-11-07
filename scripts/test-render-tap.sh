#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
w=$(mktemp -d); trap 'rm -rf "$w"' EXIT INT TERM
v=1.2.3
: > "$w/SHA256SUMS"
i=0
for f in fluxwire-$v-aarch64-apple-darwin.dmg fluxwire-$v-x86_64-apple-darwin.dmg \
  fluxwire-cli-$v-aarch64-apple-darwin.tar.gz fluxwire-cli-$v-x86_64-apple-darwin.tar.gz \
  fluxwire-cli-$v-aarch64-unknown-linux-musl.tar.gz fluxwire-cli-$v-x86_64-unknown-linux-musl.tar.gz; do
  i=$((i + 1)); printf '%064d  %s\n' "$i" "$f" >> "$w/SHA256SUMS"
done
sh "$here/render-tap.sh" "v$v" "$w/SHA256SUMS" "$w/tap"
grep -q 'version "1.2.3"' "$w/tap/Casks/fluxwire.rb" || { echo "FAIL: cask version"; exit 1; }
grep -q "$(printf '%064d' 1)" "$w/tap/Casks/fluxwire.rb" || { echo "FAIL: cask sha"; exit 1; }
grep -q "$(printf '%064d' 6)" "$w/tap/Formula/fluxwire-cli.rb" || { echo "FAIL: formula sha"; exit 1; }
grep -v fluxwire-cli-$v-x86 "$w/SHA256SUMS" > "$w/short"
if sh "$here/render-tap.sh" "$v" "$w/short" "$w/t2" 2>/dev/null; then echo "FAIL: missing checksum accepted"; exit 1; fi
echo "render-tap tests passed"

#!/bin/sh
# Renders the Homebrew cask and formula for a release.
#
#   scripts/render-tap.sh VERSION SHA256SUMS OUT_DIR
#
# Writes OUT_DIR/Casks/fluxwire.rb and OUT_DIR/Formula/fluxwire-cli.rb with the version and the
# checksums of the matching artifacts substituted. Fails if a placeholder stays unfilled.
set -eu
[ $# -eq 3 ] || { echo "usage: render-tap.sh VERSION SHA256SUMS OUT_DIR" >&2; exit 64; }
version="${1#v}"; sums="$2"; out="$3"
here=$(cd "$(dirname "$0")/.." && pwd)

sed_args="-e s/@VERSION@/$version/g"
for key in \
  "AARCH64_APPLE_DARWIN_DMG fluxwire-$version-aarch64-apple-darwin.dmg" \
  "X86_64_APPLE_DARWIN_DMG fluxwire-$version-x86_64-apple-darwin.dmg" \
  "AARCH64_APPLE_DARWIN_CLI fluxwire-cli-$version-aarch64-apple-darwin.tar.gz" \
  "X86_64_APPLE_DARWIN_CLI fluxwire-cli-$version-x86_64-apple-darwin.tar.gz" \
  "AARCH64_UNKNOWN_LINUX_MUSL_CLI fluxwire-cli-$version-aarch64-unknown-linux-musl.tar.gz" \
  "X86_64_UNKNOWN_LINUX_MUSL_CLI fluxwire-cli-$version-x86_64-unknown-linux-musl.tar.gz"; do
  placeholder="${key%% *}"; file="${key#* }"
  sum=$(awk -v n="$file" '$2 == n || $2 == "*" n { print $1 }' "$sums")
  [ -n "$sum" ] || { echo "render-tap: $file is not in $sums" >&2; exit 1; }
  sed_args="$sed_args -e s/@SHA_${placeholder}@/$sum/g"
done

mkdir -p "$out/Casks" "$out/Formula"
# shellcheck disable=SC2086
sed $sed_args "$here/packaging/homebrew/fluxwire.rb.in" > "$out/Casks/fluxwire.rb"
# shellcheck disable=SC2086
sed $sed_args "$here/packaging/homebrew/fluxwire-cli.rb.in" > "$out/Formula/fluxwire-cli.rb"
if grep -q '@[A-Z_0-9]*@' "$out/Casks/fluxwire.rb" "$out/Formula/fluxwire-cli.rb"; then
  echo "render-tap: unfilled placeholder" >&2; exit 1
fi

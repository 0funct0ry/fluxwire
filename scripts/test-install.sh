#!/bin/sh
# Exercises install.sh against a fake release served from a local directory (file://).
# Builds tiny stand-in archives with the real layout; no network, no Rust build.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM
V=9.9.9
OS=$(uname -s); ARCH=$(uname -m)
case "$ARCH" in arm64|aarch64) ARCH=aarch64 ;; *) ARCH=x86_64 ;; esac
rel="$work/rel/v$V"; mkdir -p "$rel"

case "$OS" in
  Darwin) APP_TRIPLE="$ARCH-apple-darwin"; CLI_TRIPLE="$APP_TRIPLE" ;;
  Linux) APP_TRIPLE="$ARCH-unknown-linux-gnu"; CLI_TRIPLE="$ARCH-unknown-linux-musl" ;;
esac

# CLI archive
c="$work/stage/fluxwire-cli-$V-$CLI_TRIPLE"; mkdir -p "$c"
printf '#!/bin/sh\necho fluxwire-cli test\n' > "$c/fluxwire-cli"; chmod +x "$c/fluxwire-cli"
tar -czf "$rel/fluxwire-cli-$V-$CLI_TRIPLE.tar.gz" -C "$work/stage" "fluxwire-cli-$V-$CLI_TRIPLE"

# App archive (Linux only; the macOS dmg path needs a real disk image)
if [ "$OS" = Linux ]; then
  a="$work/stage/fluxwire-$V-$APP_TRIPLE"; mkdir -p "$a/bin" "$a/share/applications"
  printf '#!/bin/sh\necho fluxwire test\n' > "$a/bin/fluxwire"; chmod +x "$a/bin/fluxwire"
  echo "[Desktop Entry]" > "$a/share/applications/fluxwire.desktop"
  tar -czf "$rel/fluxwire-$V-$APP_TRIPLE.tar.gz" -C "$work/stage" "fluxwire-$V-$APP_TRIPLE"
fi

sums() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}
( cd "$rel" && sums ./* | sed 's#\./##' > SHA256SUMS )

export FLUXWIRE_BASE_URL="file://$work/rel"
export FLUXWIRE_PREFIX="$work/prefix"

fail() { echo "FAIL: $*" >&2; exit 1; }

sh "$here/install.sh" --cli-only --version "$V" >/dev/null
[ -x "$work/prefix/bin/fluxwire-cli" ] || fail "cli not installed"
"$work/prefix/bin/fluxwire-cli" | grep -q "fluxwire-cli test" || fail "cli does not run"

if [ "$OS" = Linux ]; then
  sh "$here/install.sh" --version "v$V" >/dev/null
  [ -x "$work/prefix/bin/fluxwire" ] || fail "app not installed"
  [ -f "$work/prefix/share/applications/fluxwire.desktop" ] || fail "desktop file not installed"
fi

# A tampered archive must be refused.
echo tampered >> "$rel/fluxwire-cli-$V-$CLI_TRIPLE.tar.gz"
if sh "$here/install.sh" --cli-only --version "$V" >/dev/null 2>"$work/err"; then fail "tampered archive accepted"; fi
grep -q "checksum mismatch" "$work/err" || fail "no checksum error reported"

sh "$here/install.sh" --help | grep -q "Uninstall" || fail "help lacks uninstall instructions"
echo "install.sh tests passed"

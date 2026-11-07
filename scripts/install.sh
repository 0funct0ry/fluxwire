#!/bin/sh
# Fluxwire installer. Served at https://0funct0ry.github.io/fluxwire/install.sh
#
#   curl -fsSL https://0funct0ry.github.io/fluxwire/install.sh | sh
#   curl -fsSL https://0funct0ry.github.io/fluxwire/install.sh | sh -s -- --cli-only --version 1.0.0
#
# Installs the app (/Applications on macOS, ~/.local on Linux) and the CLI (~/.local/bin).
# Every download is checked against the release's SHA256SUMS before anything is installed.
# FLUXWIRE_BASE_URL overrides the download location (used by tests).
set -eu

REPO="0funct0ry/fluxwire"
BASE_URL="${FLUXWIRE_BASE_URL:-https://github.com/$REPO/releases/download}"
PREFIX="${FLUXWIRE_PREFIX:-$HOME/.local}"
APP_DIR="${FLUXWIRE_APP_DIR:-/Applications}"
VERSION=""
CLI_ONLY=0

usage() {
  cat <<USAGE
Usage: install.sh [--cli-only] [--version X.Y.Z] [--help]

  --cli-only       install only fluxwire-cli (no GPUI libraries needed)
  --version X.Y.Z  install that release instead of the latest
  --help           show this help

Uninstall:
  macOS:  rm -rf /Applications/Fluxwire.app ~/.local/bin/fluxwire-cli
  Linux:  rm -f ~/.local/bin/fluxwire ~/.local/bin/fluxwire-cli ~/.local/share/applications/fluxwire.desktop
USAGE
}

say() { printf '%s\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --cli-only) CLI_ONLY=1 ;;
    --version) [ $# -ge 2 ] || die "--version needs a value"; VERSION="$2"; shift ;;
    --version=*) VERSION="${1#--version=}" ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; die "unknown option: $1" ;;
  esac
  shift
done

need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required"; }
need curl
need tar
need uname

OS=$(uname -s)
ARCH=$(uname -m)
case "$ARCH" in
  arm64|aarch64) ARCH=aarch64 ;;
  x86_64|amd64) ARCH=x86_64 ;;
  *) die "unsupported architecture: $ARCH" ;;
esac
case "$OS" in
  Darwin) APP_TRIPLE="$ARCH-apple-darwin"; CLI_TRIPLE="$APP_TRIPLE" ;;
  Linux) APP_TRIPLE="$ARCH-unknown-linux-gnu"; CLI_TRIPLE="$ARCH-unknown-linux-musl" ;;
  *) die "unsupported OS: $OS (on Windows use the MSI from the GitHub Releases page)" ;;
esac

if [ -z "$VERSION" ]; then
  url=$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") \
    || die "could not look up the latest release; pass --version"
  VERSION="${url##*/v}"
fi
VERSION="${VERSION#v}"
[ -n "$VERSION" ] || die "could not determine the version"
RELEASE_URL="$BASE_URL/v$VERSION"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  die "sha256sum or shasum is required"
fi

say "Fluxwire $VERSION ($OS $ARCH)"
curl -fsSL "$RELEASE_URL/SHA256SUMS" -o "$TMP/SHA256SUMS" || die "could not download SHA256SUMS from $RELEASE_URL"

# fetch <file name>: download into $TMP and verify against SHA256SUMS.
fetch() {
  name="$1"
  say "Downloading $name"
  curl -fSL --progress-bar "$RELEASE_URL/$name" -o "$TMP/$name" || die "download failed: $name"
  want=$(awk -v n="$name" '$2 == n || $2 == "*" n { print $1 }' "$TMP/SHA256SUMS")
  [ -n "$want" ] || die "$name is not listed in SHA256SUMS"
  got=$(sha256 "$TMP/$name")
  [ "$want" = "$got" ] || die "checksum mismatch for $name (expected $want, got $got)"
}

install_cli() {
  name="fluxwire-cli-$VERSION-$CLI_TRIPLE.tar.gz"
  fetch "$name"
  tar -xzf "$TMP/$name" -C "$TMP"
  mkdir -p "$PREFIX/bin"
  cp "$TMP/fluxwire-cli-$VERSION-$CLI_TRIPLE/fluxwire-cli" "$PREFIX/bin/fluxwire-cli"
  chmod +x "$PREFIX/bin/fluxwire-cli"
  say "Installed fluxwire-cli to $PREFIX/bin"
}

install_app_macos() {
  name="fluxwire-$VERSION-$APP_TRIPLE.dmg"
  fetch "$name"
  mnt="$TMP/mnt"
  mkdir -p "$mnt"
  hdiutil attach "$TMP/$name" -mountpoint "$mnt" -nobrowse -quiet || die "could not mount the disk image"
  rm -rf "$APP_DIR/Fluxwire.app"
  cp -R "$mnt/Fluxwire.app" "$APP_DIR/" || { hdiutil detach "$mnt" -quiet || true; die "could not copy to $APP_DIR (try sudo, or set FLUXWIRE_APP_DIR)"; }
  hdiutil detach "$mnt" -quiet || true
  say "Installed Fluxwire.app to $APP_DIR"
}

install_app_linux() {
  name="fluxwire-$VERSION-$APP_TRIPLE.tar.gz"
  fetch "$name"
  tar -xzf "$TMP/$name" -C "$TMP"
  src="$TMP/fluxwire-$VERSION-$APP_TRIPLE"
  mkdir -p "$PREFIX/bin" "$PREFIX/share"
  cp "$src/bin/fluxwire" "$PREFIX/bin/fluxwire"
  chmod +x "$PREFIX/bin/fluxwire"
  cp -R "$src/share/." "$PREFIX/share/"
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$PREFIX/share/applications" >/dev/null 2>&1 || true
  command -v update-mime-database >/dev/null 2>&1 && update-mime-database "$PREFIX/share/mime" >/dev/null 2>&1 || true
  say "Installed fluxwire to $PREFIX/bin"
}

if [ "$CLI_ONLY" -eq 0 ]; then
  case "$OS" in
    Darwin) install_app_macos ;;
    Linux) install_app_linux ;;
  esac
fi
install_cli

case ":$PATH:" in
  *":$PREFIX/bin:"*) ;;
  *) say "Add $PREFIX/bin to your PATH to run fluxwire-cli." ;;
esac
say "Done. To uninstall, run: sh install.sh --help"

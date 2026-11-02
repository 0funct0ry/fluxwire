#!/usr/bin/env bash
# Starts the dev-mcp app, runs the given scenario files against it over the socket and stops
# the app. Used by `make smoke-ui` and the CI smoke jobs. Linux CI wraps it in xvfb-run.
#   scripts/smoke-ui.sh [--no-capture] fixtures/scenarios/m02-collections.yaml ...
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
features="${SMOKE_FEATURES:-dev-mcp}"
capture=()
files=()
for a in "$@"; do
  case "$a" in
    --no-capture) capture=(--no-capture) ;;
    *) files+=("$a") ;;
  esac
done
[ "${#files[@]}" -gt 0 ] || { echo "usage: $0 [--no-capture] SCENARIO.yaml..." >&2; exit 64; }

sock="$(mktemp -d)/fw.sock"
out="${SMOKE_OUT:-target/smoke}"
mkdir -p "$out"

cargo build -p fluxwire --features "$features"
target="${CARGO_TARGET_DIR:-target}/debug/fluxwire"
"$target" --mcp-socket "$sock" fixtures/payments-api >"$out/app.log" 2>&1 &
app=$!
trap 'kill "$app" 2>/dev/null || true' EXIT

for _ in $(seq 1 100); do
  [ -S "$sock" ] && break
  kill -0 "$app" 2>/dev/null || { cat "$out/app.log"; echo "app exited early" >&2; exit 2; }
  sleep 0.2
done
[ -S "$sock" ] || { cat "$out/app.log"; echo "socket never appeared" >&2; exit 2; }

status=0
for f in "${files[@]}"; do
  "$target" mcp scenario "$f" --socket "$sock" "${capture[@]}" --out "$out/$(basename "$f" .yaml)" || status=$?
done
exit "$status"

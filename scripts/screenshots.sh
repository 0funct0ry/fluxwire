#!/usr/bin/env bash
# Captures documentation screenshots from the real app (dev-mcp build) into site/public/screenshots/.
# Runs the scenarios below, which carry `screenshot` checkpoints, then copies the PNGs by scenario name.
# Needs a GPU-capable session (on Linux wrap in xvfb-run). Usage: make screenshots
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
out="${SMOKE_OUT:-target/screenshots}"
dest="site/public/screenshots"
mkdir -p "$dest"
scenarios=(fixtures/scenarios/m03-send.yaml fixtures/scenarios/m04-environments.yaml fixtures/scenarios/m09-folder-run.yaml fixtures/scenarios/m08-websocket.yaml)
SMOKE_OUT="$out" scripts/smoke-ui.sh "${scenarios[@]}"
n=0
while IFS= read -r png; do
  scenario="$(basename "$(dirname "$png")")"
  cp "$png" "$dest/${scenario}-$(basename "$png")"
  n=$((n + 1))
done < <(find "$out" -name '*.png' | sort)
echo "copied $n screenshots to $dest"

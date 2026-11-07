#!/bin/sh
# Startup timing harness (SPEC §20, ADR-023): runs the app N times with FLUXWIRE_STARTUP_TIMING=1,
# takes the median of the reported startup_ms and fails if it exceeds BUDGET_MS * TOLERANCE.
#
#   scripts/startup-timing.sh [path/to/fluxwire]    BUDGET_MS=300 TOLERANCE=2 RUNS=5
set -eu
bin="${1:-target/release/fluxwire}"
runs="${RUNS:-5}"
budget="${BUDGET_MS:-300}"
tolerance="${TOLERANCE:-1}"
[ -x "$bin" ] || { echo "startup-timing: $bin not found; build with make build-release" >&2; exit 2; }

times=""
i=0
while [ "$i" -lt "$runs" ]; do
  ms=$(FLUXWIRE_STARTUP_TIMING=1 "$bin" 2>&1 >/dev/null | sed -n 's/^startup_ms=//p' | tail -n 1)
  [ -n "$ms" ] || { echo "startup-timing: run $i reported no startup_ms" >&2; exit 2; }
  times="$times $ms"
  i=$((i + 1))
done
median=$(echo "$times" | tr ' ' '\n' | sed '/^$/d' | sort -n | awk '{ a[NR] = $1 } END { print a[int((NR + 1) / 2)] }')
limit=$(awk -v b="$budget" -v t="$tolerance" 'BEGIN { printf "%d", b * t }')
echo "startup runs (ms):$times  median=$median  limit=$limit (budget $budget x tolerance $tolerance)"
[ "$median" -le "$limit" ] || { echo "startup-timing: median $median ms exceeds $limit ms" >&2; exit 1; }

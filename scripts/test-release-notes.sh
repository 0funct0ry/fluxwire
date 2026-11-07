#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$(printf '%s\n' '[M10] Headless CLI' '[M9.01] Workspace lifecycle' '[M10] CLI exit codes' 'Fix typo' '[M9] Folder runs' \
  | sh "$here/release-notes.sh" -)
echo "$out" | grep -q '^## M10$' || { echo "FAIL: no M10 heading"; exit 1; }
[ "$(echo "$out" | grep -c '^- ')" -eq 5 ] || { echo "FAIL: item count"; exit 1; }
order=$(echo "$out" | grep '^## ' | tr '\n' ' ')
[ "$order" = "## M9 ## M9.01 ## M10 ## Other changes " ] || { echo "FAIL: order: $order"; exit 1; }
echo "release-notes tests passed"

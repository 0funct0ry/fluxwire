#!/bin/sh
# Release notes grouped by milestone ID, from commit subjects such as "[M12] OpenAPI two-way sync".
#
#   scripts/release-notes.sh [FROM_TAG] [TO_REF]      (git log FROM..TO; no FROM means everything)
#   git log --format=%s | scripts/release-notes.sh -  (read subjects from stdin; used by tests)
#
# Subjects without a [Mn] tag are listed under "Other changes".
set -eu

if [ "${1:-}" = "-" ]; then
  subjects=$(cat)
else
  from="${1:-}"; to="${2:-HEAD}"
  if [ -n "$from" ]; then range="$from..$to"; else range="$to"; fi
  subjects=$(git log --no-merges --format=%s "$range")
fi

printf '%s\n' "$subjects" | awk '
  /^[[:space:]]*$/ { next }
  match($0, /^\[M[0-9.]+\]/) {
    id = substr($0, RSTART + 1, RLENGTH - 2)
    title = substr($0, RLENGTH + 1); sub(/^[[:space:]]+/, "", title)
    if (!(id in seen)) { seen[id] = 1; order[++n] = id }
    items[id] = items[id] "- " title "\n"
    next
  }
  { other = other "- " $0 "\n" }
  END {
    # Milestone IDs sort numerically: M9 < M9.01 < M10.
    for (i = 1; i <= n; i++) for (j = i + 1; j <= n; j++) {
      a = substr(order[i], 2); b = substr(order[j], 2)
      if (a + 0 > b + 0) { t = order[i]; order[i] = order[j]; order[j] = t }
    }
    for (i = 1; i <= n; i++) printf "## %s\n\n%s\n", order[i], items[order[i]]
    if (other != "") printf "## Other changes\n\n%s", other
  }'

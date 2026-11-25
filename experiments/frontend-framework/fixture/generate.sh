#!/usr/bin/env bash
# Writes the fixture diffs from this repository's own history. Both ends are
# fixed commits on `main`, so the output is the same on every machine with a
# full clone.
#   change-1.patch  755 files, 21,210 added and 14,021 removed lines, 2.5 MB
#   change-1-alt.patch  the same head against an older base: the stub API takes
#                   "a new revision of a file" from it.
set -euo pipefail
cd "$(dirname "$0")"
head=386525cd2
git diff 1f2f4a231 "$head" > change-1.patch
git diff 0777309bf "$head" > change-1-alt.patch
wc -c change-1.patch change-1-alt.patch

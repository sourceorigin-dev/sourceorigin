#!/usr/bin/env bash
# Times a production build and a type check of each app, three times, taking
# turns. Stop the production servers first: a build replaces what they serve.
set -euo pipefail
# A dot for the decimals, whatever the machine is set to.
export LC_ALL=C
cd "$(dirname "$0")/.."

seconds() {
  local started=$EPOCHREALTIME
  "$@" > /dev/null 2>&1
  awk -v a="$started" -v b="$EPOCHREALTIME" 'BEGIN { printf "%.1f", b - a }'
}

for run in 1 2 3; do
  echo "build      react  $(cd react && seconds pnpm exec vite build) s   svelte $(cd svelte && seconds pnpm exec vite build) s"
done
for run in 1 2 3; do
  echo "type check react  $(cd react && seconds pnpm exec tsc --noEmit) s   svelte $(cd svelte && seconds pnpm exec svelte-check --tsconfig ./tsconfig.json) s"
done

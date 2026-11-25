#!/usr/bin/env bash
# Counts the lines of each part of the two apps: all of them, then those left
# without blank lines and comments.
set -euo pipefail
cd "$(dirname "$0")/.."

code() {
  cat "$@" | grep -vE '^\s*$' | grep -vE '^\s*(//|/\*|\*)' |
    awk '/^\s*<!--/{skip=1} !skip{print} /-->/{skip=0}' | wc -l
}
part() {
  local name=$1
  shift
  printf '%-46s %4s lines, %4s of code\n' "$name" "$(cat "$@" | wc -l)" "$(code "$@")"
}

echo 'The wrapper around Pierre (what we would maintain)'
part '  React' react/src/pierre/ServerFiles.tsx react/src/pierre/jsx.d.ts
part '  Svelte' svelte/src/lib/pierre/CodeView.svelte svelte/src/lib/pierre/FileTree.svelte svelte/src/lib/pierre/ServerFiles.svelte
for file in react/src/pierre/* svelte/src/lib/pierre/*; do part "    $file" "$file"; done
echo 'The review screen'
part '  React' react/src/review/Review.tsx react/src/review/Thread.tsx 'react/src/routes/changes.$change.tsx'
part '  Svelte' svelte/src/lib/review/Review.svelte svelte/src/lib/review/ThreadView.svelte svelte/src/lib/review/CommentForm.svelte 'svelte/src/routes/changes/[change]/+page.svelte' 'svelte/src/routes/changes/[change]/+page.server.ts'
echo 'The settings page'
part '  React (Base UI)' react/src/routes/settings.tsx
part '  Svelte (Bits UI and our Field)' svelte/src/routes/settings/+page.svelte svelte/src/routes/settings/+page.ts svelte/src/lib/Field.svelte
echo 'Setup: routing, the policy, the document, the build and the server'
part '  React' react/src/router.tsx react/src/start.ts react/src/routes/__root.tsx react/src/routes/index.tsx react/vite.config.ts react/server.js
part '  Svelte' svelte/src/hooks.server.ts svelte/src/routes/+layout.svelte svelte/src/routes/+page.svelte svelte/src/app.html svelte/vite.config.ts svelte/server.js
echo 'Shared by both, and no framework in it'
part '  review helpers, server loader, policy' shared/src/review.ts shared/src/server.ts shared/src/csp.ts
part '  stylesheet' shared/src/style.css
part '  generated API client' $(find shared/src/api -name '*.ts')
part '  stub API (Rust)' api/src/main.rs api/src/patch.rs

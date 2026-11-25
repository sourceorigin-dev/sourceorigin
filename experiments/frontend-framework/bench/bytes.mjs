// Lists the scripts one page loads, largest first: `node bench/bytes.mjs <url>`.
import { gzipSync } from 'node:zlib';

import { chromium } from 'playwright';

const url = process.argv[2];
const browser = await chromium.launch();
const page = await browser.newPage();
const responses = [];
page.on('response', (response) => responses.push(response));
await page.goto(url, { waitUntil: 'load' });
await page.waitForTimeout(2500);
const scripts = [];
for (const response of responses) {
  if (!(response.headers()['content-type'] ?? '').includes('javascript')) continue;
  const body = await response.body().catch(() => null);
  if (body == null) continue;
  const text = body.toString('utf8');
  // What a file holds, by the names that survive minifying.
  const holds = [
    [/react-dom|\breactjs\.org|react\.dev\/errors/, 'React'],
    [/__TSR|tanstack|TanStack/, 'TanStack'],
    [/base-ui|BaseUI/, 'Base UI'],
    [/svelte\.dev\/e\/|svelte-announcer|sveltekit/, 'Svelte or SvelteKit'],
    [/data-bits-|bits-ui/, 'Bits UI'],
    [/floating-ui|computePosition/, 'Floating UI'],
    [/diffs-container|data-diffs/, 'Pierre diffs'],
    [/file-tree-container|data-file-tree/, 'Pierre trees'],
    [/shiki|oniguruma|vscode-textmate/i, 'Shiki'],
    [/preact|__k\b.*__e\b/, 'Preact'],
    [/hey-api|serverSentEvents|querySerializer/, 'API client'],
  ].filter(([pattern]) => pattern.test(text)).map(([, label]) => label);
  scripts.push({ file: new URL(response.url()).pathname.split('/').pop(), raw: body.length, gzip: gzipSync(body, { level: 9 }).length, holds: holds.join(', ') });
}
scripts.sort((a, b) => b.gzip - a.gzip);
for (const script of scripts.slice(0, 14)) {
  console.log(`${String(script.raw).padStart(8)} raw ${String(script.gzip).padStart(7)} gzip  ${script.file.padEnd(34)} ${script.holds}`);
}
const total = (key) => scripts.reduce((sum, script) => sum + script[key], 0);
console.log(`${String(total('raw')).padStart(8)} raw ${String(total('gzip')).padStart(7)} gzip  in ${scripts.length} scripts`);
await browser.close();

// Measures the development loop of both candidates: `node bench/devloop.mjs`.
// For each: how long the dev server takes to show the review page from a cold
// start, then how long an edit takes to show in the open page, for a small
// component and for the screen's own component, and what the edit cost the
// page (a reload, the scroll position, the files already loaded).
// Stop both dev servers first: this starts its own.
import { spawn } from 'node:child_process';
import { readFileSync, rmSync, writeFileSync } from 'node:fs';

import { chromium } from 'playwright';

const root = new URL('..', import.meta.url).pathname;
const APPS = {
  react: {
    port: 4201,
    caches: ['react/node_modules/.vite', 'react/.tanstack'],
    leaf: ['react/src/review/Thread.tsx', '<strong>{comment.author}</strong>', '<strong>{comment.author} wrote</strong>'],
    screen: ['react/src/review/Review.tsx', 'files reviewed', 'files checked'],
  },
  svelte: {
    port: 4202,
    caches: ['svelte/node_modules/.vite', 'svelte/.svelte-kit'],
    leaf: ['svelte/src/lib/review/ThreadView.svelte', '<strong>{comment.author}</strong>', '<strong>{comment.author} wrote</strong>'],
    screen: ['svelte/src/lib/review/Review.svelte', 'files reviewed', 'files checked'],
  },
};
const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const browser = await chromium.launch();
const results = {};

for (const [name, app] of Object.entries(APPS)) {
  const base = `http://localhost:${app.port}`;
  const result = (results[name] = {});

  /** Starts the dev server and waits until the review page has CodeView in it. */
  async function start(label) {
    const started = performance.now();
    const server = spawn('pnpm', ['dev'], { cwd: `${root}${name}`, stdio: 'ignore', detached: true });
    let answered;
    for (;;) {
      const response = await fetch(`${base}/changes/1`).catch(() => null);
      if (response?.ok) {
        await response.text();
        answered = performance.now() - started;
        break;
      }
      await sleep(50);
    }
    const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await context.newPage();
    await page.goto(`${base}/changes/1`);
    await page.waitForFunction(() => performance.getEntriesByName('review:codeview').length > 0, null, { timeout: 120000 });
    result[label] = { serverAnswersMs: Math.round(answered), pageUsableMs: Math.round(performance.now() - started) };
    return { server, context, page };
  }
  // The whole process group: pnpm starts vite as a child.
  const stop = async ({ server, context }) => {
    await context.close();
    process.kill(-server.pid);
    await sleep(800);
  };

  for (const cache of app.caches) rmSync(`${root}${cache}`, { recursive: true, force: true });
  await stop(await start('coldStart'));
  const running = await start('warmStart');
  const { page } = running;
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message.slice(0, 160)));
  page.on('console', (message) => message.type() === 'error' && errors.push(message.text().slice(0, 160)));
  await page.waitForFunction(() => performance.getEntriesByName('review:all-files').length > 0, null, { timeout: 120000 });
  await sleep(1500);

  for (const kind of ['leaf', 'screen']) {
    const [file, before, after] = app[kind];
    const path = `${root}${file}`;
    const original = readFileSync(path, 'utf8');
    const times = [];
    let kept;
    try {
      for (let edit = 0; edit < 6; edit++) {
        // Scroll a little, keeping the first thread in view, and mark the window,
        // to see what the edit costs.
        await page.evaluate(() => {
          window.__alive = true;
          document.querySelector('.review-diff:not(.server-files)').scrollTop = 60;
        });
        await sleep(600);
        const text = edit % 2 === 0 ? after : before;
        const needle = text.includes('wrote') ? ' wrote' : text.replace(/<[^>]+>/g, '').replace('{comment.author}', '');
        const wanted = edit % 2 === 0;
        const started = performance.now();
        writeFileSync(path, edit % 2 === 0 ? original.replace(before, after) : original);
        await page.waitForFunction(
          ([part, present]) => {
            const texts = [document.body.innerText];
            for (const host of document.querySelectorAll('diffs-container')) texts.push(host.innerText);
            return texts.join('\n').includes(part) === present;
          },
          [kind === 'leaf' ? ' wrote' : 'files checked', wanted],
          { timeout: 60000, polling: 20 }
        );
        times.push(performance.now() - started);
        await sleep(1200);
        kept = await page.evaluate(() => ({
          noReload: window.__alive === true,
          scrollTop: Math.round(document.querySelector('.review-diff:not(.server-files)')?.scrollTop ?? -1),
          scrollHeight: Math.round(document.querySelector('.review-diff:not(.server-files)')?.scrollHeight ?? -1),
          serverFilesBack: document.querySelectorAll('.server-files').length,
        }));
      }
    } finally {
      writeFileSync(path, original);
    }
    // The first edit of a file pays for anything not compiled yet.
    result[`${kind}Edit`] = { firstMs: Math.round(times[0]), medianOfRestMs: Math.round(median(times.slice(1))), after: kept };
  }
  result.errorsAfterEdits = [...new Set(errors)];
  await stop(running);
}

await browser.close();
writeFileSync(new URL('./results/devloop.json', import.meta.url), `${JSON.stringify(results, null, 2)}\n`);
console.log(JSON.stringify(results, null, 2));

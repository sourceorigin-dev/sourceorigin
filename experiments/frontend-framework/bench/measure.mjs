// Measures both candidates' production builds: `node bench/measure.mjs [runs]`.
// Each run loads the review page in a fresh browser context (cold cache),
// scrolls the diff with a synthetic gesture and records every frame, then jumps
// around it. The two candidates take turns, run by run, so whatever else the
// machine is doing lands on both. bench/results keeps what was recorded.
import { mkdirSync, writeFileSync } from 'node:fs';
import { brotliCompressSync, gzipSync } from 'node:zlib';

import { chromium } from 'playwright';

import { loadavg } from 'node:os';

const runs = Number(process.argv[2] ?? '9');
const TARGETS = { react: 'http://127.0.0.1:4301', svelte: 'http://127.0.0.1:4302' };
const VIEWPORT = { width: 1440, height: 900 };

const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
};
const percentile = (values, p) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))];
};
const round = (value) => Math.round(value * 10) / 10;
const spread = (values) => ({ median: round(median(values)), min: round(Math.min(...values)), max: round(Math.max(...values)) });

const browser = await chromium.launch();

/** One cold load of a page, with the times of what appeared and what was fetched. */
async function load(base, path, { cpu = 1, waitFor = [] } = {}) {
  const context = await browser.newContext({ viewport: VIEWPORT });
  const page = await context.newPage();
  const session = await context.newCDPSession(page);
  if (cpu > 1) await session.send('Emulation.setCPUThrottlingRate', { rate: cpu });
  const responses = [];
  page.on('response', (response) => responses.push(response));
  await page.addInitScript(() => {
    // Whether a rendered file with diff lines covers the middle of the diff's
    // viewport, or only space the virtualizer has not filled yet. Not a hit
    // test: CodeView turns pointer events off while it scrolls.
    window.__covered = (root) => {
      const rect = root.getBoundingClientRect();
      const middle = rect.top + rect.height / 2;
      for (const file of root.querySelectorAll('diffs-container')) {
        const box = file.getBoundingClientRect();
        // The 12 px between two files counts as covered.
        if (box.top - 12 <= middle && box.bottom >= middle) {
          return file.shadowRoot?.querySelector('[data-column-number]') != null;
        }
      }
      return false;
    };
    window.__paint = {};
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) window.__paint.lcp = entry.startTime;
    }).observe({ type: 'largest-contentful-paint', buffered: true });
    window.__long = [];
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) window.__long.push([entry.startTime, entry.duration]);
    }).observe({ type: 'longtask', buffered: true });
  });
  await page.goto(`${base}${path}`, { waitUntil: 'load' });
  for (const mark of waitFor) {
    await page.waitForFunction((label) => performance.getEntriesByName(label).length > 0, mark, { timeout: 60000 });
  }
  await page.waitForTimeout(1500);
  const times = await page.evaluate(() => {
    const navigation = performance.getEntriesByType('navigation')[0];
    const mark = (label) => performance.getEntriesByName(label)[0]?.startTime;
    return {
      ttfb: navigation.responseStart,
      htmlDone: navigation.responseEnd,
      domContentLoaded: navigation.domContentLoadedEventEnd,
      fcp: performance.getEntriesByName('first-contentful-paint')[0]?.startTime,
      lcp: window.__paint.lcp,
      codeview: mark('review:codeview'),
      allFiles: mark('review:all-files'),
      // Main-thread time in tasks over 50 ms, beyond their first 50 ms.
      blocking: window.__long.reduce((total, [, duration]) => total + Math.max(0, duration - 50), 0),
    };
  });
  const bytes = { html: [0, 0, 0], js: [0, 0, 0], css: [0, 0, 0], font: [0, 0, 0], other: [0, 0, 0] };
  const files = { js: 0 };
  for (const response of responses) {
    const url = new URL(response.url());
    if (url.origin !== new URL(base).origin) continue;
    const body = await response.body().catch(() => null);
    if (body == null) continue;
    const type = response.headers()['content-type'] ?? '';
    const kind = type.includes('html') ? 'html' : type.includes('javascript') ? 'js' : type.includes('css') ? 'css' : type.includes('font') ? 'font' : 'other';
    if (kind === 'js') files.js += 1;
    bytes[kind][0] += body.length;
    bytes[kind][1] += gzipSync(body, { level: 9 }).length;
    bytes[kind][2] += brotliCompressSync(body).length;
  }
  return { page, session, context, times, bytes, files };
}

/**
 * Jumps the diff to a far place, as a click in the file tree does, and reports
 * how long the viewport stayed empty.
 */
async function jump(page, fraction) {
  return page.evaluate(async (target) => {
    const root = document.querySelector('.review-diff');
    const started = performance.now();
    root.scrollTop = root.scrollHeight * target;
    for (;;) {
      await new Promise((resolve) => requestAnimationFrame(resolve));
      if (window.__covered(root)) return performance.now() - started;
      if (performance.now() - started > 5000) return 5000;
    }
  }, fraction);
}

/** Scrolls the diff with a gesture at a steady speed and reports the frames. */
async function scroll(page, session, { distance, speed }) {
  const box = await page.locator('.review-diff').first().boundingBox();
  await page.evaluate(() => {
    const frames = [];
    const blanks = [];
    let last = performance.now();
    window.__scroll = { frames, blanks, running: true, long: [] };
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) window.__scroll.long.push(entry.duration);
    }).observe({ type: 'longtask' });
    const root = document.querySelector('.review-diff');
    const tick = (now) => {
      frames.push(now - last);
      last = now;
      blanks.push(window.__covered(root) ? 0 : 1);
      if (window.__scroll.running) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  const started = performance.now();
  await session.send('Input.synthesizeScrollGesture', {
    x: box.x + box.width / 2,
    y: box.y + box.height / 2,
    yDistance: -distance,
    speed,
    gestureSourceType: 'mouse',
  });
  const elapsed = performance.now() - started;
  const result = await page.evaluate(() => {
    window.__scroll.running = false;
    const root = document.querySelector('.review-diff');
    return { ...window.__scroll, scrollTop: root.scrollTop };
  });
  const frames = result.frames.slice(1);
  return {
    seconds: round(elapsed / 1000),
    scrolled: Math.round(result.scrollTop),
    frames: frames.length,
    fps: round((frames.length / elapsed) * 1000),
    frameMs: { mean: round(frames.reduce((a, b) => a + b, 0) / frames.length), p95: round(percentile(frames, 95)), p99: round(percentile(frames, 99)), max: round(Math.max(...frames)) },
    // Frames that took longer than one and a half refreshes at 60 Hz, and longer than three.
    over25ms: round((frames.filter((frame) => frame > 25).length / frames.length) * 100),
    over50ms: round((frames.filter((frame) => frame > 50).length / frames.length) * 100),
    blankFrames: round((result.blanks.reduce((a, b) => a + b, 0) / result.blanks.length) * 100),
    longTasks: result.long.length,
    longTaskMs: Math.round(result.long.reduce((a, b) => a + b, 0)),
  };
}

const names = Object.keys(TARGETS);
const output = Object.fromEntries(names.map((name) => [name, {}]));
const machine = { runs, started: new Date().toISOString(), loadAverageAtStart: loadavg().map(round) };
const each = async (run) => {
  for (const name of names) await run(name, TARGETS[name]);
};

// The first requests pay for the server starting its highlighter: not measured.
await each(async (name, base) => {
  for (let warm = 0; warm < 3; warm++) await fetch(`${base}/changes/1`).then((response) => response.text());
});
const server = { react: [], svelte: [] };
for (let run = 0; run < 20; run++) {
  await each(async (name, base) => {
    const started = performance.now();
    await fetch(`${base}/changes/1`).then((response) => response.text());
    server[name].push(performance.now() - started);
  });
}
for (const name of names) output[name].serverRenderMs = spread(server[name]);

for (const cpu of [1, 4]) {
  const collected = Object.fromEntries(names.map((name) => [name, { samples: [], scrolls: [], jumps: [] }]));
  for (let run = 0; run < runs; run++) {
    await each(async (name, base) => {
      const into = collected[name];
      const loaded = await load(base, '/changes/1', { cpu, waitFor: ['review:all-files', 'review:codeview'] });
      into.samples.push(loaded.times);
      into.bytes = loaded.bytes;
      into.files = loaded.files;
      // Let the first highlights finish, so the scroll measures scrolling.
      await loaded.page.waitForTimeout(2500);
      // Fast enough to cross many files a second, as a reader skimming a long diff does.
      into.scrolls.push(await scroll(loaded.page, loaded.session, { distance: 60000, speed: 6000 }));
      for (const fraction of [0.3, 0.6, 0.9, 0.45, 0.1]) {
        into.jumps.push(await jump(loaded.page, fraction));
        await loaded.page.waitForTimeout(400);
      }
      await loaded.context.close();
    });
  }
  for (const name of names) {
    const { samples, scrolls, jumps, bytes, files } = collected[name];
    const pick = (key) => spread(samples.map((sample) => sample[key]));
    const scrolled = (read) => spread(scrolls.map(read));
    output[name][`review, cpu x${cpu}`] = {
      load: Object.fromEntries(['ttfb', 'fcp', 'lcp', 'domContentLoaded', 'allFiles', 'codeview', 'blocking'].map((key) => [key, pick(key)])),
      scroll: {
        fps: scrolled((s) => s.fps),
        meanFrameMs: scrolled((s) => s.frameMs.mean),
        p95FrameMs: scrolled((s) => s.frameMs.p95),
        p99FrameMs: scrolled((s) => s.frameMs.p99),
        maxFrameMs: scrolled((s) => s.frameMs.max),
        over25ms: scrolled((s) => s.over25ms),
        over50ms: scrolled((s) => s.over50ms),
        blankFrames: scrolled((s) => s.blankFrames),
        longTasks: scrolled((s) => s.longTasks),
        longTaskMs: scrolled((s) => s.longTaskMs),
      },
      jumpToContentMs: spread(jumps),
      ...(cpu === 1 ? { bytes, jsFiles: files.js } : {}),
    };
  }
}

for (const path of ['/settings', '/']) {
  const collected = Object.fromEntries(names.map((name) => [name, { samples: [] }]));
  for (let run = 0; run < runs; run++) {
    await each(async (name, base) => {
      const loaded = await load(base, path);
      collected[name].samples.push(loaded.times);
      collected[name].bytes = loaded.bytes;
      collected[name].files = loaded.files;
      await loaded.context.close();
    });
  }
  for (const name of names) {
    const { samples, bytes, files } = collected[name];
    const pick = (key) => spread(samples.map((sample) => sample[key]));
    output[name][path] = { load: { ttfb: pick('ttfb'), fcp: pick('fcp'), domContentLoaded: pick('domContentLoaded'), blocking: pick('blocking') }, bytes, jsFiles: files.js };
  }
}

await browser.close();
machine.loadAverageAtEnd = loadavg().map(round);
mkdirSync(new URL('./results/', import.meta.url), { recursive: true });
writeFileSync(new URL('./results/measure.json', import.meta.url), `${JSON.stringify({ machine, ...output }, null, 2)}\n`);

// The table: median, then the range, for React and for Svelte.
const show = (value) => (value == null ? '-' : `${value.median} (${value.min} to ${value.max})`);
const line = (label, read) => console.log(`${label.padEnd(30)} ${show(read(output.react)).padEnd(30)} ${show(read(output.svelte))}`);
console.log(`load average ${machine.loadAverageAtStart.join(' ')} at start, ${machine.loadAverageAtEnd.join(' ')} at end; ${runs} runs each`);
console.log(`${''.padEnd(30)} ${'React'.padEnd(30)} Svelte`);
line('server render ms', (o) => o.serverRenderMs);
for (const cpu of [1, 4]) {
  const key = `review, cpu x${cpu}`;
  console.log(`-- ${key}`);
  for (const metric of Object.keys(output.react[key].load)) line(`load ${metric} ms`, (o) => o[key].load[metric]);
  for (const metric of Object.keys(output.react[key].scroll)) line(`scroll ${metric}`, (o) => o[key].scroll[metric]);
  line('jump to content ms', (o) => o[key].jumpToContentMs);
}
for (const path of ['/settings', '/']) {
  console.log(`-- ${path}`);
  for (const metric of Object.keys(output.react[path].load)) line(`load ${metric} ms`, (o) => o[path].load[metric]);
}
for (const path of ['review, cpu x1', '/settings', '/']) {
  console.log(`-- bytes ${path} (raw, gzip, brotli)`);
  for (const kind of ['html', 'js', 'css', 'font']) {
    console.log(`${kind.padEnd(30)} ${output.react[path].bytes[kind].join(', ').padEnd(30)} ${output.svelte[path].bytes[kind].join(', ')}`);
  }
  console.log(`${'scripts'.padEnd(30)} ${String(output.react[path].jsFiles).padEnd(30)} ${output.svelte[path].jsFiles}`);
}

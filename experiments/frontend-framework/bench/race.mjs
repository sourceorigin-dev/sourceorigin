// Loads the review page many times and reports how often, and when, CodeView
// took over from the server's files: `node bench/race.mjs <base-url> [loads]`.
import { chromium } from 'playwright';

const [base, loads = '25'] = process.argv.slice(2);
const browser = await chromium.launch();
const times = [];
let missing = 0;
for (let run = 0; run < Number(loads); run++) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  await page.goto(`${base}/changes/1`);
  const mark = await page
    .waitForFunction(() => performance.getEntriesByName('review:codeview')[0]?.startTime, null, { timeout: 6000 })
    .then((handle) => handle.jsonValue(), () => null);
  if (mark == null) {
    missing += 1;
    console.log(`run ${run}: no CodeView after 6 s`, await page.evaluate(() => ({
      serverFiles: document.querySelectorAll('.server-files').length,
      files: document.querySelectorAll('.review-diff:not(.server-files) diffs-container').length,
      rows: document.querySelector('.review-diff:not(.server-files) diffs-container')?.shadowRoot?.querySelectorAll('[data-column-number]').length,
      allFiles: performance.getEntriesByName('review:all-files').length,
    })));
  } else times.push(Math.round(mark));
  await context.close();
}
times.sort((a, b) => a - b);
console.log(`${times.length} of ${loads} took over; ms: min ${times[0]}, median ${times[Math.floor(times.length / 2)]}, max ${times.at(-1)}; missing ${missing}`);
await browser.close();

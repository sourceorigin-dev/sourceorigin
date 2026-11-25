// Opens a page, prints what the console and the Content Security Policy said,
// and saves a screenshot: `node bench/look.mjs <url> <png> [wait-ms] [dark]`.
import { chromium } from 'playwright';

const [url, png, wait = '2500', scheme = 'light'] = process.argv.slice(2);
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, colorScheme: scheme });
const seen = new Map();
const note = (line) => seen.set(line, (seen.get(line) ?? 0) + 1);
page.on('console', (message) => {
  const text = message.text();
  // Policy violations are counted below, by directive.
  if (text.includes('Content Security Policy')) return;
  if (['error', 'warning'].includes(message.type())) note(`${message.type()}: ${text.slice(0, 400)}`);
});
await page.addInitScript(() => {
  window.__violations = {};
  document.addEventListener('securitypolicyviolation', (event) => {
    const where = (event.sourceFile ?? '').split('/').pop()?.split('?')[0] || 'markup';
    const key = `${event.effectiveDirective} blocked ${event.blockedURI} from ${where}`;
    window.__violations[key] = (window.__violations[key] ?? 0) + 1;
  });
});
page.on('pageerror', (error) => note(`pageerror: ${error.message.slice(0, 400)}`));
page.on('requestfailed', (request) => note(`failed: ${request.url()} ${request.failure()?.errorText}`));
await page.goto(url, { waitUntil: 'load' });
await page.waitForTimeout(Number(wait));
for (const [key, count] of Object.entries(await page.evaluate(() => window.__violations))) note(`csp x${count}: ${key}`);
for (const [line, count] of seen) console.log(`${count}x ${line}`);
await page.screenshot({ path: png });
await browser.close();

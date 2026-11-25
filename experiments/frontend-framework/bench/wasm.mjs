// Says whether the page and a worker may compile WebAssembly under the policy,
// and which highlighter Pierre's pool ended up with: `node bench/wasm.mjs <base-url>`.
import { chromium } from 'playwright';

const browser = await chromium.launch();
const page = await browser.newPage();
await page.goto(`${process.argv[2]}/changes/1`);
await page.waitForFunction(() => performance.getEntriesByName('review:codeview').length > 0);
const probe = `try { new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])); 'allowed' } catch (error) { 'blocked: ' + error.message }`;
console.log('page:', await page.evaluate(probe));
for (const worker of page.workers()) console.log('worker:', worker.url().split('/').pop(), await worker.evaluate(probe));
await browser.close();

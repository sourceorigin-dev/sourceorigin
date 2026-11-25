// The same walk through both candidates: `node bench/check.mjs <base-url>`.
// It uses only what the two share, the text and the stylesheet's classes, so
// one script proves both do the same things. Restart the stub API first
// (bench/api.sh): the walk writes comments and marks.
import { AxeBuilder } from '@axe-core/playwright';
import { chromium } from 'playwright';

const base = process.argv[2];
const API = 'http://127.0.0.1:4100';
const SECOND_FILE = '.agents/skills/cleanup/references/agent-brief.md';
const failures = [];

async function step(name, run) {
  const started = performance.now();
  try {
    await run();
    console.log(`ok    ${name} (${Math.round(performance.now() - started)} ms)`);
  } catch (error) {
    failures.push(name);
    console.log(`FAIL  ${name}: ${String(error.message).split('\n')[0]}`);
  }
}
const assert = (condition, message) => {
  if (!condition) throw new Error(message);
};
/** Asserts something that becomes true a moment after what caused it. */
async function eventually(check, message) {
  for (let tries = 0; tries < 40; tries++) {
    if (await check()) return;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(message);
}

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await context.addInitScript(() => {
  window.__violations = [];
  document.addEventListener('securitypolicyviolation', (event) =>
    window.__violations.push(`${event.effectiveDirective} ${event.blockedURI}`)
  );
  // Errors the browser reports without throwing, such as a ResizeObserver loop.
  window.__reported = [];
  window.addEventListener('error', (event) => window.__reported.push(event.message));
});
const page = await context.newPage();
page.setDefaultTimeout(8000);
const errors = [];
// A refused request is logged by the browser itself; the walk refuses one on purpose.
page.on('console', (message) => {
  if (message.type() === 'error' && !message.text().startsWith('Failed to load resource')) {
    errors.push(message.text().slice(0, 200));
  }
});
page.on('pageerror', (error) => errors.push(error.message.slice(0, 200)));
const simulate = (body) =>
  fetch(`${API}/changes/1/simulate`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });

await step('the server sends the tree, diff lines and a thread as HTML', async () => {
  const response = await fetch(`${base}/changes/1`);
  const html = await response.text();
  assert(response.headers.get('content-security-policy')?.includes("default-src 'none'"), 'no CSP header');
  assert(html.includes('agent-brief.md'), 'the tree is not in the HTML');
  assert(html.includes('Synced from the sourceorigin-standard crate'), 'no diff line in the HTML');
  assert(html.includes('Does this still hold when the caller retries'), 'no thread in the HTML');
});

await step('without scripts, the page shows the diff', async () => {
  const still = await browser.newContext({ javaScriptEnabled: false, viewport: { width: 1440, height: 900 } });
  const plain = await still.newPage();
  await plain.goto(`${base}/changes/1`);
  await plain.getByText('Synced from the sourceorigin-standard crate').first().waitFor();
  await plain.getByText('Does this still hold when the caller retries').first().waitFor();
  await still.close();
});

await step('CodeView takes over from the server files', async () => {
  await page.goto(`${base}/changes/1`);
  await page.waitForFunction(() => performance.getEntriesByName('review:codeview').length > 0);
  await page.locator('.server-files').waitFor({ state: 'detached' });
  await page.getByText('Synced from the sourceorigin-standard crate').first().waitFor();
});

await step('all 755 files arrive', async () => {
  await page.waitForFunction(() => performance.getEntriesByName('review:all-files').length > 0, null, {
    timeout: 30000,
  });
});

await step('reply to a thread', async () => {
  const thread = page.locator('.thread').first();
  await thread.getByLabel('Reply').fill('Checked it: the retry holds.');
  await thread.getByRole('button', { name: 'Reply' }).click();
  await thread.getByText('Checked it: the retry holds.').waitFor();
  await eventually(async () => (await thread.getByLabel('Reply').inputValue()) === '', 'the reply box kept its text');
});

await step('start a thread on a line', async () => {
  await page.locator('[data-column-number="14"]').first().hover();
  await page.locator('[data-utility-button]').first().click();
  const draft = page.getByLabel('Comment on this line');
  await draft.waitFor();
  assert(await draft.evaluate((node) => node === node.getRootNode().activeElement), 'the draft is not focused');
  await draft.fill('Should this say where the crate lives?');
  await page.getByRole('button', { name: 'Comment', exact: true }).click();
  await page.locator('.thread', { hasText: 'Should this say where the crate lives?' }).getByLabel('Reply').waitFor();
  // The thread can arrive over the event stream a moment before the request answers.
  await eventually(async () => (await draft.count()) === 0, 'the draft stayed');
});

await step('a thread arrives from someone else', async () => {
  await simulate({ kind: 'thread', path: SECOND_FILE });
  const notice = page.locator('.notice.live');
  await notice.getByText(`tomas commented on ${SECOND_FILE}`).waitFor();
  await notice.getByRole('button', { name: 'Show' }).click();
  await page.locator('.thread', { hasText: 'Arrived while you were reading' }).waitFor();
});

await step('mark a file as reviewed', async () => {
  // The file's own checkbox, clicked once: the file folds as it is marked, so
  // "the first checkbox" names another file a moment later.
  const file = page.locator('diffs-container', {
    has: page.locator('[data-diffs-header]', { hasText: SECOND_FILE }),
  });
  await file.getByLabel('Reviewed', { exact: true }).click();
  await page.locator('.progress strong', { hasText: /^1$/ }).waitFor();
  assert(await file.getByLabel('Reviewed', { exact: true }).isChecked(), 'the mark did not stay');
  const saved = await (await fetch(`${API}/changes/1`)).json();
  const marked = saved.files.filter((entry) => entry.reviewed).map((entry) => entry.path);
  assert(marked.length === 1 && marked[0] === SECOND_FILE, `the API has ${JSON.stringify(marked)}`);
});

await step('a new version of a file arrives', async () => {
  await simulate({ kind: 'revision' });
  const notice = page.locator('.notice.live');
  await notice.getByText(/pushed a new version of/).waitFor();
  await notice.getByRole('button', { name: 'Show' }).click();
  await notice.waitFor({ state: 'detached' });
});

await step('the tree scrolls the diff to a file', async () => {
  await page.getByRole('treeitem', { name: /nextest\.toml/ }).click();
  await page.locator('[data-diffs-header]', { hasText: '.config/nextest.toml' }).first().waitFor();
});

// axe names an element inside a shadow root by a path of selectors, so a path
// means the element is Pierre's: its trees and diffs are the only shadow roots.
const summary = (violations, inShadow) =>
  violations
    .map((violation) => [violation.id, violation.nodes.filter((node) => Array.isArray(node.target[0]) === inShadow).length])
    .filter(([, count]) => count > 0)
    .map(([id, count]) => `${id} x${count}`)
    .join(', ');

await step('the review page passes axe, outside Pierre\'s components', async () => {
  const { violations } = await new AxeBuilder({ page }).analyze();
  console.log(`      inside Pierre's components: ${summary(violations, true) || 'nothing'}`);
  assert(summary(violations, false) === '', summary(violations, false));
});

// The review page's reported errors, read before the walk leaves it.
const reported = await page.evaluate(() => window.__reported ?? []);

await step('settings: the server refuses a field, then saves', async () => {
  await page.goto(`${base}/settings`);
  const email = page.getByLabel('Email', { exact: true });
  // Until the scripts run, submitting is the browser's own GET, in both
  // candidates: try until the page answers with the server's refusal.
  await eventually(async () => {
    await email.fill('mira@example.org');
    await page.getByRole('button', { name: 'Save settings' }).click();
    return page.getByText('This address cannot receive mail.').waitFor({ timeout: 400 }).then(() => true, () => false);
  }, 'the refusal never showed');
  assert((await email.getAttribute('aria-invalid')) === 'true', 'the field is not marked invalid');
  const described = await email.getAttribute('aria-describedby');
  assert(described != null && described !== '', 'the error is not tied to its field');
  await email.fill('mira@sourceorigin.dev');
  await page.getByLabel('Display name').fill('Mira O.');
  await page.getByRole('button', { name: 'Save settings' }).click();
  await page.getByText('Settings saved.').waitFor();
  const saved = await (await fetch(`${API}/account`)).json();
  assert(saved.display_name === 'Mira O.' && saved.email === 'mira@sourceorigin.dev', 'the API did not get the form');
});

await step('settings: the dialog traps focus, closes on Escape, and confirms', async () => {
  const open = page.getByRole('button', { name: 'Sign out other browsers' });
  await open.click();
  const dialog = page.getByRole('alertdialog');
  await dialog.waitFor();
  assert((await dialog.getAttribute('aria-labelledby')) != null || (await dialog.getAttribute('aria-label')) != null, 'the dialog has no name');
  for (let presses = 0; presses < 6; presses++) await page.keyboard.press('Tab');
  assert(await dialog.evaluate((node) => node.contains(document.activeElement)), 'focus left the dialog');
  const results = await new AxeBuilder({ page }).analyze();
  assert(results.violations.length === 0, results.violations.map((v) => `${v.id} x${v.nodes.length}`).join(', '));
  await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'detached' });
  assert(await open.evaluate((node) => node === document.activeElement), 'focus did not return to the button');
  await open.click();
  await dialog.getByRole('button', { name: 'Sign out other browsers' }).click();
  await page.getByText('No other browser is signed in').waitFor();
});

await step('settings: without scripts the form still has its values', async () => {
  const still = await browser.newContext({ javaScriptEnabled: false });
  const plain = await still.newPage();
  await plain.goto(`${base}/settings`);
  assert((await plain.getByLabel('Display name').inputValue()) === 'Mira O.', 'the server did not render the value');
  await still.close();
});

await step('no console errors and no policy violations', async () => {
  const violations = await page.evaluate(() => window.__violations);
  assert(errors.length === 0, `console: ${errors.join(' | ')}`);
  // Pierre's components trip the browser's ResizeObserver loop check in both
  // candidates; it is harmless and not theirs, so it is counted, not failed.
  const loops = reported.filter((message) => message.startsWith('ResizeObserver loop'));
  console.log(`      ResizeObserver loop notices on the review page: ${loops.length}`);
  assert(reported.length === loops.length, `reported: ${[...new Set(reported)].join(' | ')}`);
  assert(violations.length === 0, `policy: ${[...new Set(violations)].join(' | ')}`);
});

await browser.close();
console.log(failures.length === 0 ? '\nall steps passed' : `\n${failures.length} failed`);
process.exit(failures.length === 0 ? 0 : 1);

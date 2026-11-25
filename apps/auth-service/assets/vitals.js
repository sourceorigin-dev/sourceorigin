/**
 * The vitals script, which every page of auth's own loads, first, so it hears
 * the errors of the scripts after it. It exports nothing, and nothing compiles
 * it: auth embeds this file as it is, under a path naming its content, and `tsc`
 * only checks its JSDoc types. Every page works without it.
 *
 * It measures how fast the page loaded, as the browser times it: the first byte
 * (TTFB), the first and the largest contentful paint (FCP, LCP), how much the
 * layout shifted (CLS, in thousandths, its worst burst of shifts) and the longest
 * click, tap or key press (an approximation of INP), each in milliseconds. A
 * page opened in a background tab reports no paint, since it painted only once
 * someone looked. It also notes the first few errors no script caught, as the
 * error's constructor's name, such as `TypeError`, and which of auth's scripts
 * it came from, never its message or its stack.
 *
 * Once, as the page is hidden or left, it posts them to auth, with the page's
 * path and nothing of its query or fragment. Auth records them against the
 * page's route, such as `/developers/clients/{id}`, and drops a report for a
 * path that isn't one of its pages.
 */

/** Where the report goes. */
const REPORT = '/telemetry/page';
/** The errors one report names at most, as auth keeps no more. */
const MAX_ERRORS = 5;
/** One of auth's scripts, `/assets/{name}.{hash}.js`, whose name is the first group. */
const SCRIPT = /\/assets\/([a-z-]+)\.[0-9a-f]{16}\.js/;

/**
 * A `layout-shift` entry, which TypeScript's DOM types don't declare yet.
 * @typedef {PerformanceEntry & { value: number, hadRecentInput: boolean }} LayoutShift
 */
/**
 * An `event` entry, whose `interactionId` is 0 for events that aren't a click,
 * a tap or a key press, such as `pointerover`.
 * @typedef {PerformanceEntry & { interactionId?: number }} EventTiming
 */

/** Shifts closer than this to the last, in milliseconds, are one burst. */
const BURST_GAP = 1000;
/** A burst lasts at most this long, in milliseconds, as CLS counts it. */
const BURST_LENGTH = 5000;
/** Whether the page started in a background tab, whose paints wait for the tab. */
const background = document.visibilityState === 'hidden';

/** @type {{ ttfb?: number, fcp?: number, lcp?: number, inp?: number, cls?: number }} */
const timings = {};
/** @type {{ name: string, script: string }[]} */
const errors = [];

const [loaded] = performance.getEntriesByType('navigation');
if (loaded instanceof PerformanceNavigationTiming) timings.ttfb = loaded.responseStart;

/**
 * Hands each observer's callback the entries still queued for it, before the
 * report is sent.
 * @type {(() => void)[]}
 */
const drains = [];

/**
 * Calls `each` with every entry of `type` the browser records, those it
 * buffered before this script ran included, if it records them at all.
 * @param {string} type
 * @param {(entry: PerformanceEntry) => void} each
 * @returns {boolean} Whether the browser records them.
 */
function observe(type, each) {
  if (!PerformanceObserver.supportedEntryTypes.includes(type)) return false;
  /** @type {PerformanceObserverInit & { durationThreshold?: number }} */
  const options = { type, buffered: true, durationThreshold: 16 };
  const observer = new PerformanceObserver((list) => {
    list.getEntries().forEach(each);
  });
  observer.observe(options);
  drains.push(() => {
    observer.takeRecords().forEach(each);
  });
  return true;
}

if (!background) {
  observe('paint', (entry) => {
    if (entry.name === 'first-contentful-paint') timings.fcp = entry.startTime;
  });
  observe('largest-contentful-paint', (entry) => {
    timings.lcp = entry.startTime;
  });
}
/** The current burst of layout shifts: when it started and ended, and its sum. */
const burst = { start: 0, end: 0, sum: 0 };
const measuresShifts = observe('layout-shift', (entry) => {
  const shift = /** @type {LayoutShift} */ (entry);
  if (shift.hadRecentInput) return;
  const time = shift.startTime;
  if (burst.sum === 0 || time - burst.end > BURST_GAP || time - burst.start > BURST_LENGTH) {
    Object.assign(burst, { start: time, sum: 0 });
  }
  burst.end = time;
  burst.sum += shift.value * 1000;
  timings.cls = Math.max(timings.cls ?? 0, burst.sum);
});
// A page seen that measures layout shifts and had none shifted by 0, not by
// nothing; one never shown says nothing of how it shifts.
if (measuresShifts && !background) timings.cls ??= 0;
observe('event', (entry) => {
  if (!(/** @type {EventTiming} */ (entry).interactionId)) return;
  timings.inp = Math.max(timings.inp ?? 0, entry.duration);
});

/**
 * Notes `error`, thrown from the script at `url`, or from the first of auth's
 * scripts its stack names, by its constructor's name only.
 * @param {unknown} error
 * @param {string} url
 */
function note(error, url) {
  if (errors.length >= MAX_ERRORS) return;
  const stack = error instanceof Error ? (error.stack ?? '') : '';
  errors.push({
    name: error instanceof Error ? error.constructor.name : 'other',
    script: (SCRIPT.exec(url) ?? SCRIPT.exec(stack))?.[1] ?? 'other'
  });
}

addEventListener('error', (event) => {
  note(event.error, event.filename);
});
addEventListener('unhandledrejection', (event) => {
  note(event.reason, '');
});

let sent = false;
/** Posts the report, once, in a request that outlives the page. */
function send() {
  if (sent) return;
  sent = true;
  for (const drain of drains) drain();
  /** @type {Record<string, number>} */
  const rounded = {};
  for (const [name, value] of Object.entries(timings)) rounded[name] = Math.round(value);
  fetch(REPORT, {
    method: 'POST',
    keepalive: true,
    headers: { 'content-type': 'application/json', 'x-sourceorigin-csrf': '1' },
    body: JSON.stringify({ path: location.pathname, ...rounded, errors })
  }).catch(() => {
    // A report that can't be sent is dropped.
  });
}

document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'hidden') send();
});
addEventListener('pagehide', send);

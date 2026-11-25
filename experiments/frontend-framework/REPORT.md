# React with TanStack Start, or SvelteKit: what the experiment found

For issue #774. Both candidates now show the same two screens, pass the same
15-step walk under the same Content Security Policy, and were measured in turn
on the same machine with the same fixture. How to run all of it is in
[README.md](README.md).

**The recommendation is SvelteKit, and it is a close call.** The two are level
on everything the review screen is judged by: time to the first diff line,
scrolling, and how long a jump takes. Pierre's code does that work in both.
What separates them is smaller: SvelteKit sends less script on every page and
needs less code for the screen, and React gets its bridge to Pierre's
components from Pierre, where we would own ours. Section 4 says what would
turn the choice around.

## 1. The measurements

Production builds, headless Chromium, nine cold loads each, the two apps taking
turns run by run. Each figure is a median, and `bench/results` has every run
with its ranges. The fixture is 755 files of this repository's own history,
21,210 lines added and 14,021 removed, a 2.5 MB patch.

Other agents were compiling on this machine for much of the work. The timings
below are from the one full run taken while it was calm; the runs under load
are used where they add something.

| | React, TanStack Start | SvelteKit |
| --- | --- | --- |
| **Time to the first diff line** | | |
| First paint, which already shows the server's three files | 168 ms | 168 ms |
| `CodeView`'s first line: the diff is live | 549 ms | 564 ms |
| All 755 files in `CodeView` | 476 ms | 478 ms |
| `CodeView`'s first line, CPU slowed four times | 2,089 ms | 1,885 ms |
| Main-thread CPU to load the page, of which script | 538 ms, 142 ms | 485 ms, 91 ms |
| The same, CPU slowed four times | 2,306 ms, 619 ms | 1,815 ms, 379 ms |
| Server's time to render the page | 28 ms | 20 ms |
| **Scrolling**: 60,000 px at 6,000 px a second | | |
| Frames a second | 58.8 | 58.3 |
| 95th and 99th percentile frame | 16.8 ms, 33.4 ms | 16.8 ms, 33.4 ms |
| Frames over 25 ms | 1.9% | 2.7% |
| Frames with an empty viewport | 0 | 0 |
| Main-thread CPU the scroll used | 3.83 s | 3.82 s |
| Frames a second in the four runs on a busy machine | 54, 56, 52, 51 | 38, 58, 58, 58 |
| Frames a second, CPU slowed four times | 24.1 | 24.8 |
| Jump to a far file until its lines show | 4.4 ms | 4.8 ms |
| **Bundle**, gzipped | | |
| Scripts of the review page | 1,221 kB | 1,153 kB |
| Scripts of the settings page | 141 kB | 65 kB |
| Scripts of the smallest page, two links | 116 kB | 44 kB |
| Main-thread script time to load the settings page | 41 ms | 9 ms |
| HTML of the review page | 95 kB | 92 kB |
| **Lines of code**, without blanks and comments | | |
| The wrapper around Pierre's components that is ours | 39 | 134 |
| The review screen | 469 | 393 |
| The settings page | 120 | 153 |
| **Development loop** | | |
| Cold start until the review page is usable | 3.5 s | 4.8 s |
| The same with caches warm | 3.2 s | 3.1 s |
| An edit to a small component shows | 43 ms | 22 ms |
| An edit to the screen's own component shows | 87 ms, state kept | 123 ms, screen rebuilt |
| Production build | 2.3 s | 6.1 s |
| Type check | 2.4 s | 3.2 s |
| Time to build it, from an empty folder to the walk passing | 20 min | 17 min |

How to read them:

- **Speed does not decide.** Every review-screen timing is inside the other's
  range. The one consistent gap is script time: SvelteKit runs about a third
  less of it while loading, which shows on a slow CPU (1.9 s against 2.1 s to
  the first live line) and not on a fast one.
- **Scrolling is measured, not judged.** A synthetic gesture scrolls the diff
  while every frame's length is recorded, with the share of frames where the
  viewport was empty and the CPU time the main thread used, which other
  programs stretch least.
- **On a calm machine the two scroll alike.** The four runs taken while other
  programs were compiling went both ways: Svelte fell to 38 frames a second in
  the busiest one, which was also before a two-line fix to its wrapper, and
  React fell to 51 and 52 in the last two while Svelte held 58. Their ranges
  overlap in every run. I would not build on either.
- **The wrapper line counts are not the whole story.** React's 39 lines sit on
  top of the 1,142-line `CodeView.tsx` Pierre ships and maintains. Svelte's 134
  are all there is, and they cover only what this screen uses.
- **"Screen rebuilt"** means SvelteKit's hot reload made the review component
  again: the diff reloaded and the scroll position was lost. React kept both.
  Edits to smaller components keep the page's state in both.
- **The build times** are both short. SvelteKit's includes bundling its Node
  server.
- **The 20 and 17 minutes** are an agent's, and React went first, so it paid
  for what both needed: the walk, the policy Pierre needs, the worker fix. I
  set 45 minutes for each before starting; neither came near it.

### The generated API client

One client for both, generated by `@hey-api/openapi-ts` from the OpenAPI
document the Rust stub prints (utoipa). One command writes typed functions and
a small fetch runtime with no dependency to install.

What felt good, in both:

- A Rust enum tagged with `type` arrives as a TypeScript union, so
  `switch (event.type)` narrows each live event with nothing written by hand.
- The server-sent events endpoint became a typed async generator:
  `for await (const event of events.stream)` worked first time, with an
  `AbortSignal` to stop it.
- A refused form is typed too: `result.error.fields` is the 422 body's shape.
- The same functions run on the server and in the browser.

What did not:

- A `text/x-diff` answer is typed `string` even when asked for as a stream, so
  both apps cast: `rest.data as unknown as ReadableStream<Uint8Array>`.
- utoipa's attribute repeats what the handler says and nothing checks the two
  agree, so the description can drift from the code. That is about the Rust
  side, and it is where "types from the database to the page" would break
  first.

Where the frameworks differ: SvelteKit wants its own `fetch` passed in, so the
browser does not ask again, and then needs two response headers named in a hook
before the first server render works (section 2). TanStack Start needs nothing,
but loses the type of the loaded data at the place it is used (section 2).

### Form and dialog components

| | React | Svelte |
| --- | --- | --- |
| Built with | Base UI 1.9 | Bits UI 2.22 |
| Dialog | `AlertDialog` | `AlertDialog` |
| Form field | `Form` and `Field`: label, help, error and the server's refusal wired to the control | None. Ours is `Field.svelte`, 35 lines |
| Under the strict policy | No violation | No violation |
| Also has, for the auth pages | One-time-code field, number field, select, combobox, toast | Pin input, select, combobox; no toast |

Both dialogs pass the same checks: the `alertdialog` role with a name, focus
kept inside, Escape closes, focus returns to the button, and axe finds nothing
on either settings page. For forms, React's ecosystem offers more: with Svelte
the choices are our own small field, or Formsnap with Superforms, which I did
not build with.

## 2. What was hard in each

### In both: Pierre's components on a server-rendered page

These cost the same in either framework. They are the answer to "how do
Pierre's components behave when a page renders on a server and then wakes up".

**`CodeView` cannot render on a server.** Its class builds DOM in a field
initialiser (`public container = document.createElement('div')`), so only the
single-file renderers and the tree have server entry points. Both apps do the
same thing, in `shared/src/server.ts`: the server renders the tree with
`preloadFileTree` and the first three files with `preloadDiffHTML`, each as a
declarative shadow root, with the threads in the slots. The browser shows that
until `CodeView` has put its first line in the page, then removes it. The first
three files' patch travels in the page's data, so `CodeView` starts without a
request. Search engines and browsers without scripts get the tree, three files
and their threads; the other 752 files exist only after scripts run.

**The server's markup must not go in the page's data.** Both frameworks send a
page's data a second time, as a script, so the page can wake up. With the
markup in the data, the review page's HTML was 1.21 MB in React and 1.22 MB in
Svelte, more than half of it the same markup twice. `shared/src/markup.ts` files
the markup in a `WeakMap` keyed by the data instead, and the components look it
up while the server renders. Both frameworks hand the loader's own object to
the components on the server, so it worked in both, and the HTML fell to
0.65 MB and 0.64 MB.

**Pierre's components need a looser Content Security Policy than Origin Auth
sends.** They write `style` attributes and `<style>` elements, in the server's
markup and from their scripts, and take no nonce: 955 attributes and 8 elements
were refused on the first load. Pages that show them need
`style-src 'self' 'unsafe-inline'`, and a nonce there would switch
`'unsafe-inline'` off. The settings page keeps the strict policy in both apps.

**`@pierre/highlights` is off until two things are done, and nothing says
so.** Pierre's default highlighter is Shiki's JavaScript engine, so `highlights`
has to be asked for, as diffshub does. It is also WebAssembly, and a policy
without `'wasm-unsafe-eval'` refuses to compile it, on the page and in each
worker, which runs under the policy its own script was served with. My first
version had neither, worked, and was slower. With both, the scroll uses 3.8 s
of main-thread CPU where Shiki's engine used 4.4 s, in both apps. It is not
free: the review page's scripts grow from about 0.7 MB to 1.2 MB gzipped.

**Vite's default bundles Pierre's worker as one 10.9 MB file** (1.97 MB
gzipped), every grammar at once, and headless Chromium then failed to load it.
`worker: { format: 'es' }` in `vite.config.ts` makes it 40 kB that loads
grammars on demand.

**`onPostRender` never fires for a file inside `CodeView`**, so neither app can
ask Pierre when the first line is in the page. Both check each frame
(`firstDiffLine` in `shared/src/review.ts`).

**Accessibility inside Pierre's components is not ours to fix.** axe finds, in
both apps and only inside Pierre's shadow roots: 91 colour-contrast failures
(the theme's addition green on white is 3.19:1), 15 tree rows whose accessible
name does not contain their text, a search input inside the tree's `tree` role,
and 5 scrollable regions a keyboard cannot reach. Our own markup passes.

**`@pierre/trees` renders with Preact inside**, whatever the page uses, so both
bundles carry Preact.

### React with TanStack Start

**A route's data loses its type where it is used.** `Route.useLoaderData()`
takes its result type from the place it is used, so passing it straight into a
typed prop or an annotated variable is never checked against the loader:

```tsx
// Compiles, though the loader returns an Account.
const wrong: { wrong: string } = Route.useLoaderData();
// So does this, whatever the loader returns.
<Review data={Route.useLoaderData()} />;
// Only an unannotated variable gets the loader's type.
const data = Route.useLoaderData();
```

It first showed as `useState(Route.useLoaderData())` giving a state typed
`() => never`. Finding the cause took about ten minutes. For a product that
wants types from the database to the page, this is a hole a lint rule would
have to close.

**The nonce for the policy is not wired, and its wiring is not in the docs the
package ships.** I found `router.options.ssr.nonce` by reading the router's
source. It takes a middleware, a context, and a function with a server half and
a browser half (`react/src/start.ts`, `react/src/router.tsx`):

```ts
const requestNonce = createIsomorphicFn()
  .server(() => getGlobalStartContext()?.nonce)
  .client(() => undefined);

export function getRouter() {
  return createRouter({ routeTree, ssr: { nonce: requestNonce() } });
}
```

**Running on Node is left to Nitro, from a nightly build.** The shipped docs say
`npm install nitro@npm:nitro-nightly@latest`. I wrote 42 lines instead
(`react/server.js`): files from `dist/client`, everything else to the fetch
handler the build exports.

**Pierre's React wrapper has rough edges of its own.** Its JSX type for
`<diffs-container>` is in a file the package does not export, so it is declared
again in `react/src/pierre/jsx.d.ts`. `renderAnnotation` is typed as a union of
tuples, which a plain `(annotation) => …` does not satisfy:

```tsx
renderAnnotation={(...[annotation]: CodeViewAnnotationArgs<ReviewAnnotation>) => …}
```

**What a loader imports reaches every page.** TanStack Start splits a route's
component into its own file and keeps its loader in the entry, so the API
client rides along on a page with two links. My first version also dragged a
30 kB piece of Pierre and Shiki onto every page that way. SvelteKit keeps each
route's `load` with the route.

**React's own tax shows in the screen's code.** The options object must keep
its identity, and the stream outlives every render, so three handlers go through
a ref that is reassigned on each render (`applyLatest`, `toItemLatest`,
`startDraftLatest` in `react/src/review/Review.tsx`). The Svelte component has
none of these: its functions are created once and see current values.

### SvelteKit

**The wrapper around `CodeView` is ours: 134 lines against 39.** It is
`svelte/src/lib/pierre/CodeView.svelte`, over the same coordinator API Pierre's
React wrapper uses. Pierre publishes which files are in the page and the
element of each; the wrapper renders the slot content and moves each piece into
its file:

```svelte
{#each snapshot?.items ?? [] as rendered (rendered.id)}
  {#if headerMetadata}
    <div {@attach into(rendered.element, HEADER_METADATA_SLOT_ID)}>
      {@render headerMetadata(rendered.item)}
    </div>
  {/if}
  {#each rendered.item.annotations ?? [] as line}
    <div {@attach into(rendered.element, rendered.instance.getAnnotationSlotName(line))}>
      {@render annotation(line, rendered.item)}
    </div>
  {/each}
{/each}
```

The review steps of the walk passed the first time they ran against it, and it
scrolls as Pierre's React wrapper does. What it cost beyond typing it:

- Svelte's compiler refuses `slot="…"` on an element that is not written inside
  a custom element ("Element with a slot='...' attribute must be a child of a
  component or a descendant of a custom element"), so the attachment sets
  `node.slot`.
- It moves nodes Svelte owns into another element. Svelte updates and removes
  them wherever they are, and that held under every test here, but it is not a
  documented pattern.
- `flushSync` may not be nested, and Pierre publishes its first snapshot while
  Svelte is still mounting, so the wrapper skips it for that one call.
- It wraps only what this screen uses. Pierre's React wrapper also has
  controlled items, selection, edit mode, the gutter slot, three more header
  slots and a list header and footer, in 1,142 lines that Pierre maintains. We
  would write each of those when the product needs it. The helper Pierre's own
  wrapper reads a file's annotations with (`getCodeViewItemRenderData`) is not
  exported; mine reads `item.annotations`, which is the same until edit mode.

**SvelteKit 3 is eight weeks old, and its ecosystem has not all caught up.**
Version 3.0 came out on 1 October 2026. It moved the configuration into the Vite
plugin, dropped `svelte.config.js` and `$lib`, and renamed several modules, so
what I knew of SvelteKit 2 was wrong in places, and I read the migration guide
first. Bits UI installs with a peer warning: its dependency `runed` 0.35 asks
for `@sveltejs/kit ^2.21`. It worked.

**The strict policy needs two fixes that are SvelteKit's own.** Its template
wraps the page in `<div style="display: contents">`, and its root component
writes the route announcer with a `style` attribute
(`src/runtime/components/root.svelte`). Both are refused under
`style-src 'self'`. The first became a class. The second needs its exact text
pinned in the policy, and the pin breaks if SvelteKit rewords it:

```ts
'style-src-attr': ['unsafe-hashes', 'sha256-S8qMpvofolR8Mpjy4kQvEm7m1q8clzU4dfDH0AmvZjo='],
```

SvelteKit's `csp` option is one policy for every page, so the looser policy of
the pages with Pierre's components is a rewrite of the header in `handle`
(`svelte/src/hooks.server.ts`). In TanStack Start the middleware just builds the
right one.

**The generated client needs two headers that SvelteKit hides.** A `load` that
uses SvelteKit's `fetch` gets its answer copied into the page, minus every
header not named in `filterSerializedResponseHeaders`. The client reads
`content-type` and `content-length`, so the first server render failed with
"Failed to get response header `content-type`":

```ts
const response = await resolve(event, {
  filterSerializedResponseHeaders: (name) => name === 'content-type' || name === 'content-length',
});
```

**adapter-node serves files outside `handle`**, so a header on a file, which a
worker's policy needs, takes a small server of our own around its handler
(`svelte/server.js`, 20 lines).

**Bits UI has no form field.** Base UI ships `Form` and `Field`, which tie the
label, the help and the server's error to the control. For Svelte I wrote
`svelte/src/lib/Field.svelte`, 35 lines, to do the same.

## 3. What I could not verify

- **Timings on a quiet machine.** Other agents compiled Rust here during the
  work. One full run was calm (load average about 4 on 16 cores) and the apps
  took turns in every run, but treat differences under about 10% as noise.
  Whether either one scrolls worse than the other on a busy machine is not
  settled.
- **Any browser but Chromium.** No Firefox, no Safari, no phone. Pierre has
  code paths of its own for Firefox and for Safari on iOS.
- **Scrolling by hand.** The gesture is synthetic, steady, and covers 5% of the
  diff's 1.1 million pixels.
- **What Svelte's wrapper does not wrap.** Controlled items, line selection as
  state, edit mode, the gutter slot, search. Selection and editing can be
  driven on Pierre's plain object without wrapping; I did not try them, in
  either app.
- **Whether Pierre keeps the coordinator API stable.** The Svelte wrapper
  stands on it. It is exported and in Pierre's API reference, where it is
  described as coordinating React's slots. `@pierre/diffs` went from 1.6.4 to
  1.7.1 between 13 and 22 November, and `@pierre/trees` 1.0.0 is two weeks old.
- **Static builds for Origin Auth's pages.** The plan embeds them in the Rust
  executable. Both frameworks say they can build static pages; neither was
  tried.
- **Why `Route.useLoaderData()` loses its type.** I saw it with TypeScript
  6.0.3 and did not find whether another version or a setting avoids it.
- **A form sent before scripts run.** In both apps it is the browser's own
  `GET`, with the fields in the address. Neither app handles it.
- **A screen reader.** The checks are axe and the keyboard.
- **A search engine.** What is verified is that the HTML holds the tree, three
  files and their threads with scripts off.
- **What was left out on purpose:** TanStack Query, React Server Components,
  SvelteKit's remote functions (still experimental), Ark UI and Formsnap.

## 4. The recommendation, and what would change it

**SvelteKit**, for these reasons, in order of weight:

1. **The feared cost was small.** A wrapper of our own around `CodeView` was
   the reason to expect React to win. It is 134 lines, the walk's review steps
   passed the first time they ran against it, and on a calm machine it scrolls
   as Pierre's does.
2. **Less script on every page.** About 70 kB less gzipped everywhere, which is
   6% of the review page and more than half of the settings page, where the
   main thread runs 9 ms of script against 41 ms. The account pages are where
   it shows.
3. **Less code, and plainer.** The review screen is 393 lines against 469, and
   has none of the refs React needs to reach current values from a stream.
4. **The page's data kept its type.** SvelteKit's generated types held
   everywhere I used them. TanStack's hole is the kind that lets a wrong shape
   through without a word.

What React does better, and it is real: Pierre maintains its wrapper, Base UI
has the form parts, hot reload keeps the review screen's state, and the build
is faster. SvelteKit 3 is also eight weeks old, and it cost more small fixes
than TanStack Start did, each a few minutes.

**What would change it to React:**

- The product needs `CodeView`'s edit mode or its other slots soon, and
  wrapping them turns out to be weeks of work, not hours. Try edit mode in the
  Svelte wrapper before committing.
- An upgrade of Pierre's packages breaks the Svelte wrapper in a way the React
  one absorbs. Bump them once before committing and see.
- Agents write most of this code and prove slower or wrong more often in
  Svelte 5 and SvelteKit 3 than in React. I had to read SvelteKit 3's migration
  guide before writing a line, because what I knew was SvelteKit 2.
- The auth port needs accessible components Bits UI does not have and we do not
  want to write.

**What would not change it:** the timings. A quieter machine or another browser
may move them, but both apps hand the same data to the same Pierre code.

Three things to carry into the real build whichever wins, because they were
found here and are not about the framework: on the pages with Pierre's
components, ask for `@pierre/highlights` and allow `'wasm-unsafe-eval'` and
inline styles; build Pierre's worker as a module; and keep server-rendered
markup out of the data the page wakes up from.

## Choices made for the experiment

The tech stack rule says to confirm a dependency before adding it. These were
picked without asking, for throwaway code, and none is decided for the product:

| Choice | Why | The alternative |
| --- | --- | --- |
| `@hey-api/openapi-ts` 0.99 for the client | Generates functions and a typed event stream, no runtime dependency | `openapi-typescript` with `openapi-fetch`, lighter, types only |
| axum, utoipa and utoipa-axum for the stub | axum is what the repository uses | aide, which reads more from the handler |
| Base UI for React | Has form parts and an answer for the policy | React Aria Components, Radix |
| Bits UI for Svelte | The most used headless library there | Ark UI, Melt |
| Playwright 1.63 and axe 4.14 | The versions the repository's browser tests use | |

Licenses: Pierre's six packages are Apache-2.0. Everything else that reaches a
browser is MIT, Apache-2.0, BSD-3-Clause or ISC.

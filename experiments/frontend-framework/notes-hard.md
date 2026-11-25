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
up while the server renders. [RESULT]

**Pierre's components need a looser Content Security Policy than Origin Auth
sends.** They write `style` attributes and `<style>` elements, in the server's
markup and from their scripts, and take no nonce: 955 attributes and 8 elements
were refused on the first load. Pages that show them need
`style-src 'self' 'unsafe-inline'`, and a nonce there would switch
`'unsafe-inline'` off. The settings page keeps the strict policy in both apps.

**The strict policy also turns `@pierre/highlights` off, silently.** It is
WebAssembly, and a policy without `'wasm-unsafe-eval'` refuses to compile it.
Pierre then falls back to Shiki's JavaScript engine and says nothing, so the
page works and is slower. A worker runs under the policy its own script was
served with, so the script files need the policy too.

**Vite's default bundles Pierre's worker as one 10.9 MB file** (1.97 MB
gzipped), every grammar at once, and headless Chromium then failed to load it.
`worker: { format: 'es' }` in `vite.config.ts` makes it 40 kB that loads
grammars on demand.

**`onPostRender` never fires for a file inside `CodeView`**, so neither app can
ask Pierre when the first line is in the page. Both check each frame
(`firstDiffLine` in `shared/src/review.ts`).

**Accessibility inside Pierre's components is not ours to fix.** axe finds, in
both apps and only inside Pierre's shadow roots: 83 colour-contrast failures
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
`npm install nitro@npm:nitro-nightly@latest`. I wrote 40 lines instead
(`react/server.js`): files from `dist/client`, everything else to the fetch
handler the build exports.

**Pierre's React wrapper has rough edges of its own.** Its JSX type for
`<diffs-container>` is in a file the package does not export, so it is declared
again in `react/src/pierre/jsx.d.ts`. `renderAnnotation` is typed as a union of
tuples, which a plain `(annotation) => …` does not satisfy:

```tsx
renderAnnotation={(...[annotation]: CodeViewAnnotationArgs<ReviewAnnotation>) => …}
```

**React's own tax shows in the screen's code.** The options object must keep
its identity, and the stream outlives every render, so three handlers go through
a ref that is reassigned on each render (`applyLatest`, `toItemLatest`,
`startDraftLatest` in `react/src/review/Review.tsx`). The Svelte component has
none of these: its functions are created once and see current values.

### SvelteKit

**The wrapper around `CodeView` is ours: [N] lines against [N].** It is
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

It worked on the first run of the walk-through and scrolls as smoothly as
Pierre's React wrapper. What it cost beyond typing it:

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
'style-src-attr': ["'unsafe-hashes'", "'sha256-S8qMpvofolR8Mpjy4kQvEm7m1q8clzU4dfDH0AmvZjo='"],
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
`svelte/src/lib/Field.svelte`, [N] lines, to do the same.

# The frontend

This page records what the frontend must do, why Topcoat goes, why SvelteKit
replaces it, and what the experiment that chose it found.

See [the plan's overview](README.md) for what each status means.

## What it must do

From piquel's [notes](notes.md) and the discussion:

- Diffs, file trees and syntax highlighting. The notes name
  [diffs.com](https://diffs.com) and [trees.software](https://trees.software),
  both from Pierre.
- Diffs must be ridiculously fast, as fast as Pierre's diffshub demo.
- Type safety from the database to the page.
- One consistent UI across every surface. piquel calls this extremely
  important.
- Performance, and ease of development: testing, build speed during
  development.

## Topcoat goes

**Decided.** Topcoat is dropped from every page that moves to the new
frontend, which is SvelteKit. Sign-in, consent and recovery stay in Origin
Auth, rendered in Rust. Whether those few pages stay on Topcoat or move to a
small Rust templater is not decided.

- Topcoat is early and experimental, and its own README says to expect
  breaking changes. piquel would rather have something with more established
  features.
- Its roadmap still lists islands as planned. Without them, a rich component
  inside a server-rendered page needs hand-written JavaScript to connect it.
- Origin Auth already has that glue: about 1,700 lines of hand-written
  scripts in `apps/auth-service/assets` (`passkey.js` 607 lines, `search.js`
  548, `dashboard.js` 281, `picture.js` 204).
- An earlier argument for keeping Topcoat in auth was that sign-in pages
  should carry little JavaScript. It was weak: passkeys need JavaScript
  anyway.
- Two frameworks would mean two component libraries, which works against one
  consistent UI.

## Moving over

**Decided.** See
[the auth port and the admin dashboard](review.md#the-auth-port-and-the-admin-dashboard)
for how this was reached.

1. Build the product app first, in SvelteKit, with a shared component library
   in `packages/ui`.
2. Then move Origin Auth's account settings, developer console and admin
   dashboard onto it. The admin dashboard becomes one app for both services.
   The account settings get a hostname of their own.
3. Sign-in, consent and recovery stay in Origin Auth, rendered on its server.
4. Origin Auth gains a JSON API for every account and admin operation, which
   the moved pages call. Being friendly to agents needs one anyway.
5. The email templates in `crates/ui` stay in Rust. They must be rendered on
   the server that sends them.

The design tokens in `crates/ui/assets/style` are plain CSS, so they stay the
single source of the look while the pages move.

The product app renders on a server. piquel calls it a must, for search
engines, above all on repository home pages.

An earlier proposal was **dropped**: building every Origin Auth page as static
files embedded in its executable. The council's three reviewers all said that
porting the sign-in pages is not earned.

## What Pierre provides

These are facts read from Pierre's repository, `pierrecomputer/pierre`, and
its documentation on 13 November 2026.

- `@pierre/diffs` was at version 1.6.4. Its entry points:

  | Entry point | What it holds |
  | --- | --- |
  | `@pierre/diffs` | Components in plain JavaScript, and utilities |
  | `@pierre/diffs/react` | React components |
  | `@pierre/diffs/edit` | The editor for edit mode |
  | `@pierre/diffs/ssr` | Rendering on a server, with highlighting |
  | `@pierre/diffs/worker` | A worker pool that highlights off the main thread |

- The plain JavaScript API has four core components. One is `CodeView`: a
  single scroll region for a list of files and diffs, with virtualization per
  line, sticky headers, selection across items, annotations, and scrolling to
  an item or a line.
- The React components are a thin layer over the plain JavaScript ones.
- `@pierre/highlights` is a highlighter written in WebAssembly text, with 152
  languages. It runs in Node, browsers and Cloudflare Workers. Pierre says it
  is about 300 times faster than Shiki.
- `@pierre/trees` exports a plain `FileTree` class with `render` and
  `hydrate`, and React hooks. Its documentation says other frameworks can wrap
  the plain class.
- Diffshub, in `apps/diffshub`, is Next.js and React, under Apache-2.0. Its
  speed does not come from React. It comes from `CodeView`, the worker pool,
  `@pierre/highlights`, and parsing the patch as a stream, file by file. Any
  framework can use all four.

The experiment confirmed that annotations can be rendered through the plain
JavaScript API, without React. Our Svelte wrapper does it.

## The experiment

**Decided: SvelteKit.** The experiment ran on 25 November 2026, as
sourceorigin#774, and piquel chose SvelteKit on its result. Its code and its
full report are on the branch `chore/frontend-framework-spike`, in
`experiments/frontend-framework`, which never merges.

It built the same two screens in React with TanStack Start and in SvelteKit:
change review, and a small account-settings form with a dialog. Both rendered
on a server, passed the same walk under the same Content Security Policy, and
were measured in turn on one machine. The fixture was 755 files of this
repository's history, a 2.5 MB patch.

The recommendation was SvelteKit, as a close call:

| | React, TanStack Start | SvelteKit |
| --- | --- | --- |
| The diff's first live line | 549 ms | 564 ms |
| The same, CPU slowed four times | 2,089 ms | 1,885 ms |
| Scrolling, frames a second | 58.8 | 58.3 |
| Scripts of the review page, gzipped | 1,221 kB | 1,153 kB |
| Scripts of the settings page, gzipped | 141 kB | 65 kB |
| Our wrapper around Pierre's components | 39 lines | 134 lines |
| The review screen | 469 lines | 393 lines |

- **Speed does not decide.** Pierre's code does the work in both, and every
  timing of the review screen is inside the other's range.
- **The feared cost was small.** A wrapper of our own around `CodeView` was
  the reason to expect React to win. It is 134 lines and scrolls as Pierre's
  React wrapper does.
- **SvelteKit sends less script on every page,** about 70 kB less gzipped.
  That is more than half of the settings page, so the account pages gain
  most.
- **A page's data kept its type in SvelteKit.** In TanStack Start,
  `Route.useLoaderData()` took its type from where it was used, so a wrong
  shape compiled.
- **What React does better:** Pierre maintains its wrapper, and Base UI has
  form parts that Bits UI lacks.

Out of the experiment: Leptos, Effect, and which framework T3 Code and
diffshub use.

## What the experiment taught

These hold for the product, whatever the framework:

- **`CodeView` can't render on a server.** The server renders the file tree
  and the first files of a diff, and the browser takes over from there. A
  search engine sees the tree, those files and their comment threads, not the
  whole diff.
- **Pierre's components need a looser Content Security Policy** than Origin
  Auth sends: inline styles, and WebAssembly for `@pierre/highlights`. So
  pages that show diffs get their own policy, and the account pages keep the
  strict one. That is one more reason the account settings have their own
  hostname.
- **`@pierre/highlights` has to be asked for,** and its WebAssembly allowed
  in the policy, on the page and in each worker. Without both, the slower
  default runs and nothing says so.
- **We own the Svelte wrapper.** It covers what the review screen uses. Line
  selection as state, edit mode and the other slots are written when the
  product needs them. It stands on an API of Pierre's that could change;
  `@pierre/diffs` went from 1.6.4 to 1.7.1 within two weeks.
- **Components:** Bits UI for dialogs and menus, with a form field of our own,
  since Bits UI has none.
- **The API client** is generated by `@hey-api/openapi-ts` from the OpenAPI
  document, and worked well in both. utoipa's attributes can drift from the
  handlers they describe, and nothing checks them: that is where types from
  the database to the page would break first.
- **Accessibility inside Pierre's components isn't ours to fix:** contrast
  failures in the theme, and regions a keyboard can't reach.
- **SvelteKit 3 is new.** It came out on 1 October 2026, and parts of its
  ecosystem still ask for version 2.
- **Not verified:** any browser but Chromium, a screen reader, and scrolling
  by hand.

## Type safety

**Proposed.** Generate the TypeScript client from the platform's OpenAPI
document. The repository already generates TypeScript types from Rust with
`ts-rs`, for the TypeScript SDK.

# Testing

This page tells you which tests to run before a pull request, which kind of test
to write for a change, and what CI checks for you.

## Before a pull request

```console
just validate
```

`just` alone runs the same recipe. It formats the code, runs Clippy and ESLint,
looks for unused code and dependencies, type-checks Rust and TypeScript, runs the
Rust unit tests and the TypeScript SDK's tests, builds the Rust docs, runs the doc
tests and audits the dependencies. It needs no database and no browser.

Then run the layers your change can affect: `just db test` when it touches SQL,
migrations or code that reads and writes the database, and the browser suite when
it changes a page, a script or a sign-in flow.

While you work, run only the tests for what you changed, such as
`just test -p auth-service`.

## The test layers

Write a test at the lowest layer that can show the behaviour: a unit test if one
can, a database test if the behaviour lives in SQL or a transaction, a browser
test only when it needs a real browser.

### Rust unit tests

```console
just test                                 # the whole workspace
just test -p auth-service -E 'test(step_up)'
```

`just test` runs the workspace's tests with
[nextest](https://nexte.st), and passes its arguments on. Unit tests live in a
`tests.rs` beside their module, as the
[Rust conventions](rust-conventions.md#documentation-and-tests) describe.

They need no network. Passkey ceremonies use a software authenticator
(`SoftPasskey`), GitHub sign-in a local HTTP server that stands in for GitHub's
API, and Google sign-in ID tokens signed with keys the test generates.

### Snapshot tests

Some output is easier to review whole than through assertions, such as every
kind of email Origin Auth sends. A snapshot test renders it and compares it with
a file under `snapshots/` beside the test, with [insta](https://insta.rs). When
the output changes on purpose, accept the new one, then review its diff with the
rest of the change:

```console
INSTA_UPDATE=always just test -p auth-service -E 'test(every_email_reads_as_its_snapshot)'
```

Without `INSTA_UPDATE`, a test whose output differs from its snapshot fails and
shows the difference.

### Database tests

```console
just db test
```

Tests that need PostgreSQL are marked `#[ignore]`, with a reason that names
`TEST_DATABASE_URL`, so `just test` skips them.
`just db test` starts a throwaway PostgreSQL 18 cluster that listens only on a
Unix socket, and a throwaway ClickHouse in Docker for auth's
[analytics tests](#analytics-tests), runs every test of `auth-service` and
`sourceorigin-repository` in them, ignored ones included, and deletes them. It then runs `just db prepare-check`,
which fails if the committed query cache doesn't match the migrated schema
exactly.

To use a database of your own instead, set `TEST_DATABASE_URL` to it. The tests
create accounts in it and leave it migrated, and create more databases on its
server, so name only a server you can throw away, never one that holds real
accounts, and connect as a role that may create databases. A TCP URL must
include `sslmode=disable` or `sslmode=verify-full`.

The databases a run creates are named `test_` and a random suffix, and the
tests never drop them. Drop them when they pile up:

```console
psql "$TEST_DATABASE_URL" -Atc "SELECT format('DROP DATABASE %I;', datname) FROM pg_database WHERE datname LIKE 'test\_%'" | psql "$TEST_DATABASE_URL"
```

A new database test starts from the fixtures in `crates/database/src/fixtures.rs`.
`Database::test()` connects with the limits every connection of the service has,
and `Database::test_config` configures a pool of your own with them.
`insert_named` creates an account, `InsertSession::lasting` describes a session,
`known_history` lists an account's events, `until_blocked` waits until other
transactions queue behind a lock a transaction holds, and `test_channels!`
declares channels whose deliveries a test inserts itself.
Another crate's tests get them through the `test-util` feature. A test in
`crates/database` that inserts rows with plain SQL connects through
`Database::test_pool()` instead, and auth-service's tests use `Replicas`,
`Browser`, and the `html`, `json` and `reply` readers in
`apps/auth-service/src/tests.rs`. A test that needs a migrated database no other
test shares gets one from `Database::isolated`.

The other tests share one database, its rows and its advisory locks, and run
under the service's own time limits: 15 seconds a statement, 5 seconds a lock
wait and 30 seconds idle in a transaction. Give a test a database of its own
when it:

- needs history or state the whole database shares, such as the audit chain's
  tests, or every administrator or alert;
- holds a lock the other tests take, for longer than they wait under those
  limits. The signing key tests do: they hold the signing key lock while they
  generate keys, and every replica a test starts waits for that lock. On a busy
  machine, a key takes seconds to generate.

A test waits longer than the service does only for what it needs to get going,
never for what it asserts. Its pools wait up to a minute for a connection
(`Database::TEST_ACQUIRE_TIMEOUT`), and the audit tests up to two minutes for
their events to be sealed, which every older transaction on the server holds
back. nextest ends a test that runs five minutes as hung, as
`.config/nextest.toml` sets.

### Analytics tests

```console
just analytics test
```

Tests that need ClickHouse are marked `#[ignore]`, with a reason that names
`TEST_ANALYTICS_URL`, and have `analytics` in their names. `just analytics test`
starts a throwaway ClickHouse in Docker, the image `infra/piqueld.toml` runs, on
a free loopback port, runs them, and deletes it. To run any command against
one, use `just analytics scratch -- <command>`. `just db test` and
`just coverage` run them too, in a scratch ClickHouse of their own unless
`TEST_ANALYTICS_URL` names one, so they need Docker. A test
that only checks which rows a change records captures them instead, with
`AnalyticsCapture` (see [analytics](architecture/analytics.md#testing)).

### SDK test vectors

The Rust and TypeScript SDKs run the same cases, in
[`tests/sdk-vectors`](https://github.com/sourceorigin-dev/sourceorigin/tree/main/tests/sdk-vectors),
so both accept and refuse the same tokens, key sets, callbacks and cookies. The
cases are generated by `crates/crypto/tests/sdk_vectors.rs`. When an SDK
behaviour must be the same in both languages, add a case there, then:

```console
just sdk-vectors
```

The [vectors' README](https://github.com/sourceorigin-dev/sourceorigin/blob/main/tests/sdk-vectors/README.md)
describes the case format.

### TypeScript SDK tests

```console
just web test    # unit tests, every vector included, with coverage
just web smoke   # the built package in Node, Bun and workerd
```

`just web test` runs `packages/auth/test/*.test.ts` with Node's test runner.
`just web smoke` builds the package, then validates one vector token with it in
Node, [Bun](https://bun.sh) and workerd, the Cloudflare Workers runtime. It needs
`bun` installed.

### Browser tests

```console
just web test-browser
```

The browser suite runs Chromium against a running deployment of auth over HTTPS:
CI's, or a [preview](getting-started.md#run-your-changes) of your checkout. Write
one for what only a browser shows: a page's layout or script, a passkey ceremony,
a sign-in that crosses origins. Its `no-javascript` project runs
`tests/browser/no-javascript.spec.ts` with JavaScript disabled, for what a page
must do without scripts. `tests/browser/accessibility.spec.ts` runs
[axe](https://github.com/dequelabs/axe-core) on auth's pages and the dashboard's,
in both themes, and fails on any WCAG 2.2 AA rule it breaks: a page you add
belongs in its list. The
[browser tests' README](https://github.com/sourceorigin-dev/sourceorigin/blob/main/tests/browser/README.md)
says how to run them.

### Generated files

Ordinary tests compare each committed generated file with what the code
generates now, so `just test` fails while one is stale. The query cache is checked
when the code compiles, and by `just db prepare-check`.
[Generated files](contributing.md#generated-files) lists each one and the recipe
that regenerates it.

## Write a focused test

- **Test one behaviour, and name the test for what it proves**, such as
  `google_key_fetch_is_a_client_span_of_its_request`. Its doc comment says why
  the behaviour matters.
- **Test it once, at the lowest layer that fails without the change**: not
  again in a wrapper, at each call site or on each surface.
- **Extend before you add.** When an existing test or table covers the
  behaviour, add a row or an assertion to it; write a new test only when no case
  fits.
- **A fix comes with a test that fails without it, written first**, when it
  fixes logic, data, state or a protocol. A purely visual fix, such as spacing,
  colour or alignment, is shown with before and after screenshots in the pull
  request instead, and gets no test.
- **Assert the outcome**, not that something ran. A test that only checks a page
  loads or a command exits 0 proves little and still has to be maintained.
- **Wait for events, not time.** No `thread::sleep` or `tokio::time::sleep` to
  let something happen, no assertions on how much real time passed, and no
  retries: await the channel, notification or state change that marks the
  milestone, such as a `watch` the test's fake sends on, or take a lock with
  `Database::exclusive` to wait for its holder. Move tokio's clock with
  `#[tokio::test(start_paused = true)]`, or with `pause`, `advance` and `resume`
  between database calls, so the clock is never paused across database I/O,
  and end an expiry PostgreSQL judges at once, as `SessionRepository::expire`
  and `RecoveryRepository::expire` do. When nothing signals the milestone, as for a
  real subprocess or network peer, a lock wait PostgreSQL shows only in its
  catalog views (`until_blocked`), or production code that reads the system
  clock, poll or sleep and say why on the line above: `// wall-clock: <why>`.
- **Don't test that a removed feature stays removed.** Once the code is gone,
  there is nothing left to break.
- **Cover in the browser whatever needs a browser.** Each spec says at its top
  what it covers. CI's deployment of auth has no request limit, so a suite can be
  as large as the behaviour needs.

## Recipes

Test recipes pass extra arguments on to nextest or Playwright, quoted filter
expressions included.

| Task | Recipe |
| --- | --- |
| Formatting, lints, the engineering standard's rules, unused code, type checks, unit tests, Rust docs, doc tests and audits | `just validate` |
| CI's quick checks, for only what changed since the merge base with `origin/main` | `just check-changed` |
| Format, or check formatting | `just fmt` / `just fmt-check` |
| Clippy and ESLint with warnings denied, and `publint` and `attw` on the built TypeScript SDK | `just lint` (`just rust lint` / `just web lint`) |
| The engineering standard's rules, from the `sourceorigin-standard` crate; `cargo xtask standard --fix` makes their mechanical fixes | `just standard` |
| Unused Rust dependencies (cargo-machete), and unused web files, exports and dependencies (knip) | `just unused` (`just rust unused` / `just web unused`) |
| Type-check Rust, and TypeScript with the pages' scripts | `just check` (`just rust check` / `just web check`) |
| Type-check each published crate with no features, then with each feature alone, as a dependent might enable it | `just rust features` (part of `just check`) |
| Rust unit tests, or one package's | `just test` / `just test -p auth-service` |
| Rust docs with warnings denied, and the doc tests | `just doc` / `just doc-test` |
| TypeScript SDK tests with coverage, and the runtime smoke test | `just web test` / `just web smoke` |
| Database tests and the query cache check, in a scratch cluster and a scratch ClickHouse (needs Docker) | `just db test` |
| Tests that need ClickHouse, in a scratch one (needs Docker) | `just analytics test` |
| Query cache check alone, against a migrated `TEST_DATABASE_URL` | `just db prepare-check` |
| Coverage per crate of every Rust test, database tests included, into `target/lcov.info` | `just coverage` |
| Any command against a scratch cluster, or a scratch ClickHouse | `just db scratch -- psql` / `just analytics scratch -- env` |
| Browser tests, against a running deployment | `just web test-browser` |
| Register the browser tests' clients with auth on `localhost` | `just web browser-client` |
| Dependency audits: cargo-deny's advisories, licenses, `*` requirements, banned crates and sources other than crates.io, and pnpm's | `just audit` (`just rust audit` / `just web audit`) |
| Validate the piqueld manifests with piquelctl, and the telemetry configs with the images that run them (needs Docker) | `just infra check` |
| Regenerate the query cache, the SDK vectors, the TypeScript SDK's wire types and shared constants, or the generated documentation | `just db prepare` / `just sdk-vectors` / `just sdk-types` / `just docs reference` |
| Build the book and check its links | `just docs` |
| Package and build the SDKs as `just publish` publishes them, without uploading | `just publish --dry-run --allow-dirty` |

## What CI runs

CI runs the same recipes, in these jobs:

| Job | What it runs |
| --- | --- |
| `checks` | `just rust fmt-check`, `just rust unused`, and `just rust audit` when dependencies change |
| `web` | `just web fmt-check`, `check`, `lint`, `unused`, `test` and `smoke`, and `just web audit` when dependencies change |
| `rust` | `just coverage` against a PostgreSQL 18 service, then `just db prepare-check` |
| `rust-lint` | `just rust lint`, `just doc` and `just doc-test` |
| `package` | `just publish --dry-run`: see [releasing](releasing.md#what-gets-published) |
| `msrv` | `cargo check` of the published crates with all their features, then `just rust features`, with their oldest supported Rust, the workspace's `rust-version` |
| `e2e` | The browser suite, against the debug binary behind Caddy, with Mailpit receiving auth's email and sourceorigin.dev's web server on its own host |
| `docker` | Builds the auth, web and docs images, the first two with the Rust `rust-toolchain.toml` selects and from the layers `main` cached last |
| `infra` | `just infra check`, with the latest piquelctl |
| `standard` | `just standard` |
| `docs` | `just docs` |

Every job but `msrv` builds with Rust stable, which `rust-toolchain.toml`
selects, as the images do. A new stable release can bring new rustc and Clippy
warnings, which then fail CI until fixed.

A pull request runs only the jobs its changes can affect, by the paths it changes:
Rust jobs for Rust code and the files it reads, such as the committed audit keys
and the generated reference pages, the web job for files Prettier, ESLint or tsc
can see, `e2e` and `docker` for anything the service, its images, the TypeScript
SDK or the browser suite is built from, and `infra` for the manifests and the
telemetry configs. Other Markdown changes run only `docs` and `standard`, which
run on every pull request: a change anywhere can break a link, and the rules
read more than the Rust sources. The `changes` job of
[ci.yml](https://github.com/sourceorigin-dev/sourceorigin/blob/main/.github/workflows/ci.yml)
reads the exact filters from
[filters.yml](https://github.com/sourceorigin-dev/sourceorigin/blob/main/.github/filters.yml),
which `just check-changed` reads too.

The merge queue and manual runs run every job. The `ci` check sums the others
up: it fails if any job failed, and passes when the rest succeeded or were
skipped. The merge queue waits for it and for `standard / pr-checks`, which
[pr.yml](https://github.com/sourceorigin-dev/sourceorigin/blob/main/.github/workflows/pr.yml)
runs: the engineering standard's checks of the pull request itself, such as its
title, its description and the exceptions it adds, which need the
`approved-exception` label from an approver.

`main` receives the exact commit the merge queue tested, so a push to `main`
doesn't test it again. It only refreshes the caches that pull requests and the
merge queue start from, since GitHub lets every branch read `main`'s caches:

- `checks` and `docs` save no cache, and `web` saves only the pnpm store,
  which `package` and `e2e` save too, so `main` skips them.
- `docker` builds the auth and web images, saving their layers. It skips the
  docs image, which caches none.
- The jobs that restore a Rust cache build only when no cache is saved under
  their current key, as after a dependency or toolchain change, or after GitHub
  evicts an unused cache. Then they build without testing. `package` and
  `msrv` run as usual, since they only build. `rust-lint` runs Clippy and
  rustdoc, and compiles the doc tests without running them. `rust` runs
  `just coverage-build` and `just db check`, `e2e` builds the binaries without
  starting the browser suite, and `infra` installs piquelctl without checking
  the manifests. `rust` and `e2e` start no Postgres or Mailpit.

Steps marked `if: *tests` or `if: *builds` in `ci.yml` are the ones `main` skips:
the first test what the queue tested, and the second build only when no cache
is saved yet. A job's Rust cache key includes its ID, so a job that needs a new
cache bumps its key's prefix. `sqlx-cli` and `piquelctl` publish no prebuilt
binaries, so `rust-cache` keeps them after it restores.

A commit's coverage report is in its merge queue run.

Coverage is reported, never enforced. `rust` prints it per crate and uploads
`target/lcov.info` as the `rust-coverage` artifact, and `just web test` prints
the TypeScript SDK's. A separate workflow audits the dependencies every Monday,
so new advisories surface without a change.

CI doesn't run:

- `just docs screenshots`, which retakes the documentation's screenshots on a
  preview; see [screenshots](documentation.md#screenshots).
- Sign-in with live Google or GitHub accounts, or Apple's and Google's password
  managers syncing passkeys between real devices. Try those by hand on a preview
  when your change touches them.

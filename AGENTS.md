# sourceorigin

SourceOrigin is the project and its site. Origin Auth is its sign-in service
(sign-in, the account settings, the developer console, the admin dashboard and
the SDKs), and a person's account is an Origin Account. Use these names in
everything people read.

This repository contains everything under the sourceorigin.dev domain:

- `apps/auth-service`: auth.sourceorigin.dev (OIDC, its sign-in pages, the account
  settings and the developer console), the admin dashboard and the operator CLI.
- `apps/web`: sourceorigin.dev's own site.
- `crates/`: `auth-sdk` (the Rust SDK), `common`, `crypto`, `database`,
  `telemetry` and `ui`.
- `packages/auth`: the TypeScript SDK.
- `infra/`: Dockerfiles and piqueld deployments.
- `tools/xtask`: `cargo xtask`, which publishes the SDKs and releases a version
  (`just publish`, `just release`).
- `tests/browser` (Playwright) and `tests/sdk-vectors` (cases every SDK passes).
- `docs/`: the documentation book, a user guide and a contributor guide.

This is still very early development. Breaking changes are expected. There are
no real users yet so breaking things doesn't matter.

## Guidelines

When adding a feature, make sure all our surfaces get the feature as
appropriate. This includes the operator CLI (`apps/auth-service/src/cli`), the
developer console, the user settings, the admin panel, the SDKs and the
documentation. All surfaces should support all features. Don't let anything be
left behind. A feature ships with its documentation in the same pull request:
use the `sourceorigin-docs` skill.

The CLI is the most complete surface. An admin operation may stay CLI-only
(`AdminApplication::page()` in `apps/auth-service/src/web/admin/mod.rs` maps
it to `None`) only with a documented reason next to its arm, as `PurgeUser`,
`ApplyClient` and `PruneAuditChain` have.

Generated files are committed; regenerate them with `just db prepare` (SQLx
cache), `just sdk-vectors`, `just sdk-types` and `just docs reference` (the CLI
and configuration references).

## Conventions

Follow `docs/dev/rust-conventions.md`, which applies the shared
[engineering standard](https://gist.github.com/PiquelChips/04330e52c03efbe9ce4c889c88673a27)
to SourceOrigin, and `docs/dev/testing.md` for which tests to write.

## Validation

`just validate` (or `just` alone) formats the code and runs the lints, type
checks, unit tests, Rust docs and audits. Run it before handing work back, plus
`just docs` when a change touches Markdown, `just db test` when it touches SQL
or the database, and the browser suite when it changes a page, a script or a
sign-in flow. While iterating, run only what you changed, such as
`just test -p auth-service`. If a step cannot run, report the exact command and
why.

## Issues, commits & pull requests

Issues live in GitHub Issues. Titles of issues and pull requests follow
Conventional Commits and say what changes for whoever uses it; pull requests
are squash-merged, so the title becomes the commit on `main`.

Keep pull requests small and focused: one change each, which a reviewer can
read in one sitting. Split refactors from behaviour changes, and stack
dependent pull requests, each on the last. Open them as drafts while
iterating. `docs/dev/contributing.md` has the rest.

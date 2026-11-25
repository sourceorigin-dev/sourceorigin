# Cleanup pass: area agent

You own one area of SourceOrigin in a parallel cleanup pass. Your task names
the area, paths, packages, branch prefix, title scope, tracking issue and
`CARGO_BUILD_JOBS`, and lists what the other agents own.

Read every file in your area, and `docs/dev/rust-conventions.md`. Make real
simplifications, not cosmetic churn, and report bigger issues. Never add an
`#[allow]`. Use `#[expect(..., reason = "...")]` on the smallest item that
needs it.

**Scope.** Do only what this skill's task asks. No drive-by refactors, renames,
reformatting or extra features. Note anything else in your report or in an issue
instead of fixing it. Ask the owner before widening the change. Reread your diff
before handing back and drop every hunk the task doesn't need. See "Scope" in
`AGENTS.md`.

## Rules

- **Change no behavior.** That includes HTTP responses (and which error a
  request with several faults gets), OIDC and OAuth wire formats, rendered
  pages and emails, CLI flags, help, output and exit codes, configuration
  variables, domain events and their audit records, log and span fields,
  metrics, timeouts, lock order, query plans, and the public API of the
  published crates (`auth-sdk`, `common`, `crypto`) and `@sourceorigin/auth`.
  `git diff <base>` shows no change to a migration, a `snapshots/` file or a
  generated file, except the `.sqlx` entry of a query you rewrote. Don't
  use blanket search and replace.
- **Report bugs instead of fixing them.** The orchestrator opens an issue and a
  fix PR for each. If a cleanup can't be done without changing behavior, report
  it too. Report a vulnerability only in your final message, never in a PR,
  commit or code comment.
- **Stay in your area.** Edit another area only when a rename forces it:
  keep it minimal and list those files in the PR description. Report what
  crosses areas: another lane cleans up between them.
- **Keep the docs references right.** If you rename something a page in
  `docs/` names, update that page (`sourceorigin-docs` skill) and run
  `just docs`. Don't reword `///` comments or clap help that feed the
  generated reference pages.
- Stop and report anything that needs a design decision.

## Checks

Export `CARGO_BUILD_JOBS` for every cargo command. For your packages, run:

- `just install`, then `just fmt`
- `cargo clippy --locked -p <pkg> --all-targets --all-features -- -D warnings`,
  and again without `--all-features`
- `RUSTDOCFLAGS='-D warnings' cargo doc --locked --no-deps --all-features -p <pkg>`,
  and `cargo test --locked --doc --all-features -p <pkg>` for a library
- `just db prepare`, if you changed a query
- `just test -p <pkg> --all-features`; for `auth-service` and
  `sourceorigin-repository`, `just db test` instead. Both fail on a stale
  generated file.
- `just rust unused`
- `cargo semver-checks -p <pkg> --baseline-rev origin/main`, for a published
  crate
- `just web check`, `just web lint`, `just web unused` and `just web test`, if
  you touched TypeScript or JavaScript
- `just docs screenshots`, if you touched a page's markup, styles or script.
  It must rewrite no image in `docs/images`. It deploys a preview, so run it
  once per PR, last.

Despite `AGENTS.md`, don't run `just validate`, `just lint` or `just test`
without `-p`: they build the whole workspace. CI runs on drafts every job a
PR's paths select, including the browser suite, the images, packaging and the
MSRV build, so leave those to it and read their results before you report.

Prove each "unchanged" claim against the PR's base: `--help` output, the
rendered pages, emails and HTTP responses, the TypeScript SDK's built type
declarations, query plans (`EXPLAIN` in `just db scratch -- psql`). Before you
call a failing check unrelated, check that it fails on `main` too. Never change
a test just to make it pass.

## Delivery

- Split your work into focused PRs: one kind of cleanup in one part of your
  area each, small enough to review in one sitting. Branch each from a
  freshly fetched `origin/main` as `<prefix><topic>`, and keep your PRs from
  sharing files. If one needs another, stack it: branch from the other PR's
  branch, open it with that branch as its base, and say so in its
  description.
- Make small, logical commits with conventional messages and no
  `Co-Authored-By` line.
- Rebase on `origin/main` (a stacked PR on its base branch), and clean up any
  new code the rebase brings in. PRs merge by squash, so when a stacked PR's
  base merges, move only its own commits with
  `git rebase --onto origin/main <old base tip>`, and check it targets
  `main`.
- Open each PR as a **draft** with `file-pr`, despite its default, titled
  `refactor(<scope>): <what got simpler>` (or `refactor: …` without a scope)
  and labelled `refactor` and the other labels that fit (`gh label list`). Its
  description says:
  - `Part of #<n>`
  - what was hard to read and how you fixed it
  - every test you merged or removed
  - files edited outside your area, and why
  - what you compared to prove the rest unchanged
- Don't mark a PR ready.

Your final message gives:

1. The PR links, in merge order, with what each one cleans up.
2. The checks you ran for each, with their results.
3. Bugs: `file:line`, what goes wrong and how to reproduce it.
4. Issues that cross areas, and other design issues: `file:line`, what's
   wrong, why it matters and a fix.

Check every item against the source before you report it. When you get review
findings, fix them on that PR's branch, rerun the affected checks, push, and
report what changed.

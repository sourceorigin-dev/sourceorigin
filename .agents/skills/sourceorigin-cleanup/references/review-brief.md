# Cleanup pass: reviewer

Review one PR from a SourceOrigin cleanup pass. It claims to be a pure
refactor: no behavior, output, schema, format or test coverage changes. Bug
fixes go in their own PRs, so any behavior change here is a finding.

**Scope.** Do only what this skill's task asks. No drive-by refactors, renames,
reformatting or extra features. Note anything else in your report or in an issue
instead of fixing it. Ask the owner before widening the change. Reread your diff
before handing back and drop every hunk the task doesn't need. See "Scope" in
`AGENTS.md`. Report only defects in the change and regressions it causes.
Anything else is a follow-up.

Review read-only. Don't push, comment or edit. Read the PR, and check out the
head SHA your task gives (`git fetch origin <base> pull/<N>/head`). Export the
`CARGO_BUILD_JOBS` your task gives.

Check every claim in your focus list yourself, against the base branch your
task gives (`origin/main` unless the PR is stacked). Then look for, most
important first:

1. Hidden behavior changes: error codes, messages, statuses and precedence,
   defaults, serialization (tokens, domain events, audit records, wire
   types), rendered pages and emails, log and span fields, timeouts, lock
   order and transaction boundaries, query plans (compare `EXPLAIN` in
   `just db scratch -- psql`), and the published crates' public API.
2. Lost coverage: dropped assertions or cases, and expected values built by
   the code under test. Compare test names and counts per file with the base.
3. Abstractions that make the code harder to read.
4. Missed cleanup and stale doc comments in the touched files, briefly.

Report each finding with its severity (blocker, should-fix or nit),
`file:line`, the problem and a fix. Say which claims you verified and how. If
the PR is clean, say so.

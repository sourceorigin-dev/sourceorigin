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

Terms that are easy to mix up:

- **App and client:** the documentation calls a site that signs people in an
  *app*; the code, the CLI (`client`) and the dashboard's **Clients** page call
  it an OIDC *client*. `AdminApplication` is the admin dashboard itself.
- **Operator and administrator:** an operator runs `auth-service` commands at a
  terminal, with the database's credentials. An administrator is an Origin
  Account holding administrator rights, acting on the dashboard.
- **Sessions:** an auth session is a browser, or an app's passkey sign-in on its
  own page (an inline session), signed in to auth. The dashboard's session hangs
  off an auth session. An app's own session is the app's, which the SDKs keep.

This is still very early development. Breaking changes are expected. There are
no real users yet so breaking things doesn't matter.

## Scope: do what was asked, nothing more

**Scope creep is the most common way an agent's work goes wrong here. Watch for
it at every step and push back on it, including when a reviewer or your own
curiosity suggests it.**

- **One task, one change.** Do the task you were given. A pull request is one
  change and holds nothing else.
- **Small enough for a reviewer to read in one sitting.** Split refactors from
  behaviour changes.
- **No drive-by changes.** No refactors, renames, reformatting, dependency
  bumps, "while I'm here" fixes or extra features the task didn't ask for.
- **Found something else?** Say so in your report, or file an issue. Don't fix
  it in this change.
- **Review findings outside the change are follow-ups.** Reviewers, council
  included, report only defects in the change and regressions it causes. A
  finding about code the change doesn't touch goes to an issue, not into the PR.
- **Ask before widening.** If doing the task well really needs a wider change,
  stop and ask the owner first, and say why.
- **Check before handing back.** Reread your diff. Every hunk must be needed for
  the task; remove any that isn't.
- **Be ambitious inside the scope.** Finish the task completely and well.
  Ambition is never a reason to widen it.

## Hazards

What you run here can reach beyond your worktree. Each rule says why, so you
can recognise a case it doesn't name.

- **Never deploy, promote or delete staging or production**, or change the
  application on piqueld (`piquelctl env …`, `app deploy`, `app apply`). They
  are the owner's, and only merges to `main` and the owner's promotions change
  them.
- **Previews:** deploy and delete only your own branch's preview, or your own
  `--slot` of it when another agent shares the branch, and pass that `--slot`
  to every preview command. Other previews belong to other agents at work;
  delete one only when the owner asks.
- **Permissions:** report a permission error to the owner. Never work around
  it with other credentials or tokens found on the machine: they grant what the
  owner chose not to give you.
- **Databases:** `just db test` and `just coverage` make a scratch cluster and
  delete it. Never point `TEST_DATABASE_URL` at a database, or a server, that
  matters: the tests migrate it and leave accounts in it, and they create
  databases and create and drop roles, which belong to the whole cluster.
  `auth-service` commands, `just auth` included, act on whatever database their
  `AUTH_DATABASE_*` variables name: run them on your preview
  (`piquelctl preview exec sourceorigin <branch> auth -- auth-service --operator <you> …`).
- **Processes:** stop only a process you started, by the PID you captured, or
  the one that owns your own port. Never kill by name or pattern (`pkill -f`,
  `killall`): your own agent process has the worktree's path in its arguments,
  and other worktrees run the same binaries.
- **Shared containers and ports:** `just auth mailpit` and
  `just infra telemetry` start Docker containers with fixed names and ports, and
  `just docs serve` takes port 3000. Another worktree may be using them: reuse
  one that is running, and don't stop it.

## Surfaces

A feature reaches every surface it applies to. Before handing one back, walk
this list and say which entries applied:

- the operator CLI (`apps/auth-service/src/cli`), the most complete surface;
- the user settings (`/settings`) and the developer console (`/developers`);
- the admin dashboard (`apps/auth-service/src/web/admin`);
- the Rust SDK (`crates/auth-sdk`) and the TypeScript SDK (`packages/auth`);
- the SDK test vectors (`tests/sdk-vectors`), when both SDKs must agree;
- for what an account holds, its history (`DomainEvent`), its data export and
  its purge (the tests of `crates/database/src/inventory` fail until a new
  table says what it holds);
- the browser tests' accessibility pass, for a new page;
- the documentation, in the same pull request: the user guide, the contributor
  guide, the screenshots and the skills in `docs/user/agents/skills`. Use the
  `sourceorigin-docs` skill.

Every way in needs a way out and a way to see it, on the same surfaces: create
needs delete, enable needs disable, grant needs revoke, and each state shows
where people look for it.

Every admin operation is an `AdminOperation`: `AdminApplication::page()`
(`apps/auth-service/src/web/admin/mod.rs`) fails to compile until it maps it to
a dashboard page or `None`, and the CLI's `every_operation_has_its_command` test
fails until a command names it. An operation may stay CLI-only (`None`) only
with a documented reason next to its arm, as `PurgeUser`, `ApplyClient` and
`PruneAuditChain` have.

## Guidelines

Generated files are committed; regenerate them with `just db prepare` (SQLx
cache), `just sdk-vectors`, `just sdk-types` and `just docs reference` (the CLI
and configuration references).

Read a dependency's real source, at the version `Cargo.lock` pins, in
`~/.cargo/registry/src` (`node_modules` for TypeScript), rather than trusting
what you remember of a fast-moving crate's API.

If a rule here fights the task, say so plainly, and get the owner's sign-off
before breaking it.

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

Each pull request is one change, as [Scope](#scope-do-what-was-asked-nothing-more)
says. Stack dependent pull requests, each on the last, and open them as drafts
while iterating. `docs/dev/contributing.md` has the rest.

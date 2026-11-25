# Analytics

This page explains how auth records analytics: typed rows, each a *signal*, kept
in ClickHouse. It tells you how to add one, and what a signal may hold.

Telemetry says what one request did, and is kept for days. Analytics counts
what many did, and is kept for months: every committed event, for example, so
a chart of sign-ins counts them exactly. Each environment has its own
ClickHouse, `clickhouse` in `infra/piqueld.toml`, beside its PostgreSQL. See
[analytics](../operations/analytics.md) to run it and read it.

## Signals

A signal is a struct declared with `signal!`, in `sourceorigin-telemetry`. The
declaration is also the table:

```rust
sourceorigin_telemetry::signal! {
    /// A domain event committed, as the audit history records it, without which
    /// account it concerns or which administrator caused it: the history keeps
    /// both, and its export and purge cover them.
    pub struct EventRecorded in "events", kept 390 days, ordered by (kind) {
        /// What happened, such as `signed_in`.
        pub kind: &'static str,
        /// The kind of actor that caused it, such as `account` or `operator`.
        pub actor: &'static str,
        /// How the person authenticated, for a sign-in, a failed one or a new
        /// account, such as `passkey`.
        pub method: Option<&'static str>,
        /// Why it happened, for a failed sign-in, a revoked grant or a failed code
        /// redemption, such as `replayed`.
        pub reason: Option<&'static str>,
        /// The event's ID in the audit history.
        pub event: EventId,
        /// The trace of the request or job that recorded it.
        pub trace: Option<TraceId>,
    }
}
```

- **The table** is named after `in`, and every field is a column. The doc
  comments become the table's and the columns' comments, so ClickHouse
  describes its own tables.
- **`time`** comes first in every table: when the row was recorded, in
  milliseconds, or the day for a [product signal](#consent).
- **`kept`** is the retention: ClickHouse deletes a row that many days after
  its time.
- **`ordered by`** names the fields rows are sorted by, before their time.
  Name what queries filter on most, fewest distinct values first. A name that
  isn't a field doesn't compile, and neither does an `Option` field, which
  ClickHouse refuses to sort by: a sort key must be a `Key`.

Record a row where it happens:

```rust
EventRecorded { kind, actor, method, reason, event, trace }.record();
```

`record` queues the row and returns at once. It never fails and never waits.

## What a signal may hold

A field's type must implement `Column`, which says the ClickHouse type that
stores it. Only types that hold no free text and no directly identifying data
implement it:

| Rust type | ClickHouse type |
| --- | --- |
| `bool`, `u8` to `u64`, `i64`, `f64` | `Bool`, `UInt8` to `UInt64`, `Int64`, `Float64` |
| `Option<u32>` | `Nullable(UInt32)` |
| `&'static str`, and its `Option` | `LowCardinality(String)`, and `LowCardinality(Nullable(String))` |
| `EventId` | `UUID` |
| `ClientId`, and its `Option` | `LowCardinality(String)`, and `LowCardinality(Nullable(String))` |
| `Option<TraceId>` | `Nullable(FixedString(32))` |
| `Option<Route>` | `LowCardinality(Nullable(String))` |
| `Instance` | `LowCardinality(String)` |

A vocabulary is a `&'static str`, such as a `str_enum!` value's `as_str()`, so
no text a user or client typed can become one. A `String`, an email address, an
IP address, a user agent or a path has no `Column`, so a signal holding one does
not compile. Neither does an account's ID, `UserId`: an account's export and
purge don't reach ClickHouse, so a signal never names an account.
`EventRecorded` leaves out the account and the administrator, which the audit
history keeps. Record only a registered client's `ClientId`, never one a
request names before auth found the client: that one is text a caller typed.

A `Route` is a route template, such as `/account/passkeys/{id}`: only
`HttpTelemetry` builds one, from the template the router matched, so it is
never a path a client sent. An `Instance` is the process's own UUID, which its
spans and logs carry as `service.instance.id`, so the rows of several replicas
can be told apart.

`Column` is sealed: only `crates/telemetry/src/analytics/mod.rs` implements
it. Add a type there only if it has the same guarantee. The guarantee holds as
long as a `&'static str` is a literal: never leak a `String` to make one.

## Consent

Most signals say how SourceOrigin runs, such as `requests` or `events`, and
every request and change records them. A few say how people use it, such as
which kind of passkey someone signed in with: *product* signals, which a
person may keep out of. Declare one as a `product struct`:

```rust
sourceorigin_telemetry::signal! {
    /// A passkey someone signed in or confirmed with, once that committed.
    pub(crate) product struct PasskeyUsed in "passkeys_used", kept 390 days, ordered by (synced) {
        // …
    }
}
```

The product signals are `passkeys_used`, which kinds of passkeys people assert
with, and `authorization_steps`, how far each app's sign-ins get: `requested`,
each time the `sign_in` or `consent` page shows, then `authorized` or
`denied`, for the redirect flow and sign-ins on an app's own page alike. A
silent `prompt=none` check involves no person and counts nothing.

It implements `Product` instead of `Signal`, and its `record` takes a
`Consent`, so a product row can't be recorded without one. A `Consent` comes
from `RequestContext::consent`, inside a transaction, or `consent_of`, before
one begins, given the signed-in account if any, and only when:

- the account didn't leave usage statistics, on its **Your data** page, which
  keeps a row in `analytics_opt_outs` (see
  [usage statistics](../../user/your-data/usage-statistics.md)); and
- the request didn't carry `Sec-GPC: 1`, the browser's
  [Global Privacy Control](https://globalprivacycontrol.org).

Before anyone signs in, auth can't know whose choice applies, so only Global
Privacy Control keeps a request out: `consent_of` takes the signed-in account,
if any, even when its session falls short of the request. Read consent before
a transaction that takes locks, never with a second connection while it holds
them.

Record a product row about a change with `DatabaseTransaction::after_commit`,
so a change that rolls back records nothing:

```rust
if let Some(consent) = context.consent(&mut tx, user_id).await? {
    tx.after_commit(move || used.record(consent));
}
```

A product row still names no account. Its time holds the day only, and the
writer sends product tables once an hour, or once 5,000 rows wait, so neither
the row nor its insert, which ClickHouse's query log times, points to the
moment of the sign-in or change it counts. An hour with a single counted
sign-in still points to it, to whoever reads ClickHouse's own logs and the
audit history together: consent decides whether a person is counted at all.

## What auth's pages report

Most rows are recorded on the server. Two come from the browser: every page of
auth's own loads the vitals script (`apps/auth-service/assets/vitals.js`) before
its other scripts, so it can measure how fast the page loaded and catch the
errors the other scripts throw:

- the time to the first byte (TTFB), the first and the largest contentful paint
  (FCP and LCP), and the cumulative layout shift (CLS), in thousandths, from
  navigation timing and `PerformanceObserver`;
- the longest interaction, from the `event` entries, as an approximation of
  Interaction to Next Paint (INP);
- each error and unhandled rejection no script caught, up to 5, as the name of
  its constructor, such as `TypeError`, and which of auth's scripts threw it.
  The script never reads the error's message or sends its stack: either can
  quote what the page shows.

Once, when the page is hidden or left, the script posts one JSON body to
`/telemetry/page`, with the CSRF header and `keepalive`, so the request
outlives the page. The body holds the page's `location.pathname`, never its
query or fragment. `VitalsApplication` (`apps/auth-service/src/vitals`)
records it:

1. It maps the path to the page route it is an instance of
   (`routes::page::template_of`), such as `/developers/clients/{id}` for an
   app's page, and drops the report when none matches. Only the template is
   recorded, never the path.
2. It reads each timing as a whole number, clamped to a minute, and the layout
   shift to 10. It keeps the first 5 errors, reads an unknown error kind as
   `other`, through its `str_enum!`, and an unknown script as `other`.
3. It records a `page_timings` row and one `script_errors` row per error, and
   answers `204` either way.

The route reads no session and queries no database. The security layer takes a
report only from auth's own pages, with the CSRF header, and gives reports a
budget of their own, `beacon`, so a page's report never costs a passkey
ceremony its `login` budget (see
[rate limit](request-lifecycle.md#rate-limit)). A body is limited to 4 KiB.

Both tables are kept 30 days. Their rows name no account and no browser, only
a page's route, so they say how auth runs, as `requests` does, and need no
consent. The admin dashboard's pages, and sourceorigin.dev's, don't load the
script. Auth collects no Content-Security-Policy violation reports and no
Network Error Logging reports.

## Tables

`auth-service migrate` brings ClickHouse to the declarations of the signals in
`ANALYTICS_TABLES` (`apps/auth-service/src/cli/mod.rs`). List a new signal
there. It creates the database if it doesn't exist, then, for each table:

1. creates the table if it doesn't exist, partitioned by month and compressed
   with ZSTD;
2. adds the columns it lacks, and updates every comment;
3. sets its retention, which applies as parts merge, without rewriting the
   table.

Running it again changes nothing. It never drops a column, changes a type or
changes a table's sort order: store a new type in a new column, and a new order
in a new table, and stop recording the old one. A renamed field is a new column
too.

A failed analytics migration doesn't fail `migrate`: it prints a warning, and
the deployment goes on. Rows for a table that doesn't match its declaration are
dropped until the next `migrate` succeeds.

Connected as ClickHouse's owner (`ANALYTICS_OWNER_URL`), `migrate` also
creates or resets the writer that `ANALYTICS_URL` names, with the
`ANALYTICS_PASSWORD` password, allowed only to insert rows: a compromised
service can add rows, but read none. One `GRANT … WITH REPLACE OPTION` sets
its rights, so running replicas never lose them while it does.

## The writer

`Telemetry::init` starts the writer when `ANALYTICS_URL` is set. Each table may
have 5,000 rows waiting, and a task sends each table's every second, a product
table's every hour, or once 5,000 wait, as one `INSERT … FORMAT JSONEachRow`
over ClickHouse's HTTP interface, so ClickHouse gets few, large inserts.
`Telemetry::shutdown` sends what is left before the process exits, product
tables first; a process that is killed loses up to an hour of product rows.

Analytics is best effort, like telemetry. When a table already has 5,000 rows
waiting, or ClickHouse refuses or doesn't answer an insert, the rows are
dropped, never retried. A table's limit is its own, so a flood of requests drops
`requests` rows, never `events`. The writer then logs `analytics rows dropped`
with how many. A request never waits for ClickHouse, and an outage costs rows,
not memory. A failed insert's log keeps only ClickHouse's error code: its
message can quote the rows.

An invalid `ANALYTICS_URL` or `ANALYTICS_PASSWORD` stops `serve` and the web
app, as broken telemetry does. Other commands warn and discard their rows.
Unset, every row is discarded.

## Reading it back

The admin dashboard's **Analytics** page reads ClickHouse through
`AdminService::analytics` (`apps/auth-service/src/admin/analytics`), as
`sourceorigin_reader`, a user `migrate` allows only to read the signals'
tables, and every table's size, with a time, memory and row limit on each
query.
`Store::query` sends a fixed statement and its values as ClickHouse query
parameters (`{name:Type}`), never as SQL text, and decodes each `JSONEachRow`
line into a struct; an error names the query. The reader's settings keep
64-bit integers unquoted in JSON, but other users' may not, so the queries
never return one. The page's queries run at once, each from the
start of the first hour or day its chart shows, in UTC, and a `Period` fills in
those that counted nothing. They cast counts to `UInt32` and sizes to
`Float64`, which a chart's `f64` holds exactly or near enough. A column whose
values are a vocabulary, such as `authorization_steps.step`, is recorded and
read through its `str_enum!`, in `sourceorigin_common::admin`, so both agree;
its `else` variant reads a spelling another release wrote, so one row never
breaks the page. A column the page only lists, such as `script_errors.name`,
is read as text and shown as written, which never breaks it either.
To chart a new signal, add its query there, and its chart to
`crates/ui/src/admin/analytics.rs`, with `crates/ui/src/charts`.

Each app's **Insights** tab, in the developer console, reads the same way,
through `OwnedClient::insights` (`apps/auth-service/src/clients/insights`).
Only `AccountService::owned_client` makes an `OwnedClient`, once it has found
the owner's own app, and the queries take the client ID from it, as a bound
parameter, so an owner reads only their app's rows. The owner already knows
who signed in, from their own callbacks, so the counts of what people did on
auth's pages, `authorization_steps`, are coarsened against differencing:

- they cover whole UTC days, up to `AppInsights::until`: a day shows once
  twice the writer's `PRODUCT_INTERVAL` has passed since it ended, so its
  last rows are in, and then every sign-in of the day at once;
- each day's counts are moved by `Noise::offset`: a secret amount under
  `FLOOR`, 5, keyed by the app and the day counted, the same for every step
  of the day, and derived from the token digest key, so every replica and
  every reload draws the same one, once for good. Each count is then rounded
  down to a multiple of 5, and the days added up (`Noise::floored`), shown as
  "About 25"; a total of none is `Few`. Rounding a moved count is unbiased,
  so the total is about right, and on any one day, a step reached less often
  never shows more than one reached more often;
- a step another release wrote shows only once it is no longer `Few`.

They count steps, not people, approximately. What remains:

- a lone sign-in on a day, all of its steps at once, shows one time in five.
  An owner who pads a day's steps with requests of their own, using a step
  that sign-in didn't reach as a reference, learns of it up to about three
  times in five, once;
- tokens are exact and count everyone, and someone who left usage statistics
  counts in no step. So an owner who compares the counts with their own
  callbacks can estimate, within about 5 a day, how many people left. One
  person's choice is one fact behind every step of every sign-in: an app
  whose only user signs in five times a day shows "About 5" or more at once
  if they're counted, and nothing if not;
- a change of token digest key draws every day still shown again, so an
  owner who noted each day's rounding gets a second one of up to 30 days;
- an app with few users still learns about how far its users get, as any
  analytics would.

`charted` (`apps/auth-service/src/charted.rs`)
holds what both pages read: `Period`, its `period!` macro, and `StepCount`.
The reader runs at most 16 queries at once, so pages that read can't take
ClickHouse over.

## Testing

`AnalyticsCapture::start()` captures the rows a test records on its thread,
instead of queuing them, as `Telemetry::capture` does for spans:

```rust
let analytics = AnalyticsCapture::start();
// … run the change …
assert!(analytics.rows("events").iter().any(|row| row["kind"] == "account_disabled"));
```

Tests that need ClickHouse itself are marked `#[ignore]`, with a reason naming
`TEST_ANALYTICS_URL`. `just analytics test` runs them in a scratch ClickHouse,
and `just db test` runs them with the dashboard's, which need PostgreSQL too:
see [testing](../testing.md#analytics-tests).

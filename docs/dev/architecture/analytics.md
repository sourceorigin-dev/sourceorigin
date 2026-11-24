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
  milliseconds.
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
have 5,000 rows waiting, and a task sends each table's every second, or once
5,000 wait, as one `INSERT … FORMAT JSONEachRow` over ClickHouse's HTTP
interface, so ClickHouse gets few, large inserts. `Telemetry::shutdown` sends
what is left before the process exits.

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

## Testing

`AnalyticsCapture::start()` captures the rows a test records on its thread,
instead of queuing them, as `Telemetry::capture` does for spans:

```rust
let analytics = AnalyticsCapture::start();
// … run the change …
assert!(analytics.rows("events").iter().any(|row| row["kind"] == "account_disabled"));
```

Tests that need ClickHouse itself are marked `#[ignore]`, with a reason naming
`TEST_ANALYTICS_URL`. `just analytics test` runs them in a scratch ClickHouse:
see [testing](../testing.md#analytics-tests).

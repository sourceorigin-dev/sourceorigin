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
    /// A domain event committed, as the audit history records it, without whom
    /// it concerns: the history keeps that, under its own retention.
    pub struct EventRecorded in "events", kept 400 days, ordered by (kind) {
        /// What happened, such as `signed_in`.
        pub kind: &'static str,
        /// The kind of actor that caused it, such as `user` or `operator`.
        pub actor: &'static str,
        /// The administrator who caused it from the admin dashboard, if one did.
        pub administrator: Option<UserId>,
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
  isn't a field doesn't compile.

Record a row where it happens:

```rust
EventRecorded { kind, actor, administrator, event, trace }.record();
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
| `UserId`, `ClientId`, `EventId`, and the `Option` of the first two | `UUID`, and `Nullable(UUID)` |
| `Option<TraceId>` | `Nullable(FixedString(32))` |

A vocabulary is a `&'static str`, such as a `str_enum!` value's `as_str()`, so
no text a user or client typed can become one. A `String`, an email address, an
IP address, a user agent or a path has no `Column`, so a signal holding one does
not compile. User IDs are pseudonymous, as in telemetry, but keep a user's ID
out of a signal unless a chart needs it: `EventRecorded` leaves out the subject
the audit history already keeps.

Add a `Column` implementation only for a type with the same guarantee, in
`crates/telemetry/src/analytics/mod.rs`.

## Tables

`auth-service migrate` brings ClickHouse to the declarations of the signals in
`ANALYTICS_TABLES` (`apps/auth-service/src/cli/mod.rs`). List a new signal
there. For each table, it:

1. creates the table if it doesn't exist, partitioned by month and compressed
   with ZSTD;
2. adds the columns it lacks, and updates every comment;
3. sets its retention, which applies as parts merge, without rewriting the
   table.

Running it again changes nothing. It never drops a column or changes a type:
store a new type in a new column, and stop recording the old one. A renamed
field is a new column too.

Connected as ClickHouse's owner (`ANALYTICS_OWNER_URL`), `migrate` also
creates or resets the writer that `ANALYTICS_URL` names, with the
`ANALYTICS_PASSWORD` password, allowed only to insert rows: a compromised
service can add rows, but read none.

## The writer

`Telemetry::init` starts the writer when `ANALYTICS_URL` is set. Rows wait in a
queue of 10,000, and a task sends each table's every second, or once 5,000
wait, as one `INSERT … FORMAT JSONEachRow` over ClickHouse's HTTP interface,
so ClickHouse gets few, large inserts. `Telemetry::shutdown` sends what is left
before the process exits.

Analytics is best effort, like telemetry. When the queue is full, or
ClickHouse refuses or doesn't answer an insert, the rows are dropped, never
retried. The writer then logs `analytics rows dropped` with how many. A
request never waits for ClickHouse, and an outage costs rows, not memory. A
failed insert's log keeps only ClickHouse's error code: its message can quote
the rows.

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

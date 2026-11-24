# Telemetry

This page explains the telemetry auth-service and sourceorigin-web produce, how to add
more, and what never goes in it. What a metric would count or measure, such as
every request's duration or every committed event, is an
[analytics](analytics.md) row instead, and a chart is a query over them.

Both services log to stderr. When `ANALYTICS_URL` names the environment's
ClickHouse, they also store their spans and logs there, in the `spans` and
`logs` tables, beside the analytics rows (see
[the analytics database](../operations/analytics.md) to read them).
`sourceorigin-telemetry` (`crates/telemetry`) does both. A feature gets most of its
telemetry for free and adds the rest in one line.

## What every feature gets

- **A trace per request, job run and delivery attempt.** `HttpTelemetry` wraps
  auth's public and admin routers and sourceorigin-web's, outside every layer that can
  refuse a request ([request lifecycle](request-lifecycle.md)). Each request gets one root span,
  named after its method and route template (`POST /oauth/token`). Every log line
  inside the span carries the trace ID, on stderr and in exported logs. Exclusive
  jobs get one root span per run, and delivery workers one per attempt. Incoming
  `traceparent` headers are ignored: a public caller must not choose the trace IDs
  that events keep.
- **An access log line** with status and duration, such as `GET
  /account/passkeys/{id}`, and a `requests` [analytics](analytics.md) row with
  the service, listener, method, route template, status, duration and trace. A
  request that matches no route logs `GET unmatched`, and its row has no route:
  its path is whatever the client sent, so it is never recorded. Health probes
  log at debug level only, and record no row.
- **A span per query.** `run!` wraps every query in a trace in a `db.query` span
  with its parameterized SQL (`db.query.text`), never its bind values. Queries
  outside a trace, such as an idle delivery worker's poll every second or a
  command-line run's, are not traced: each would be a trace of its own.
- **Events in telemetry.** Every event recorded with `DatabaseTransaction::record`
  stores the current trace ID in `domain_events.trace_id`, and once its
  transaction commits it is logged as `event recorded` (with its ID, kind, kind
  of actor, subject and, when one acted from the admin dashboard, administrator) and
  stored as an `events` [analytics](analytics.md) row, with the method and
  reason of the events that have one, such as a failed sign-in's. The audit
  history and telemetry therefore point at each other: the admin dashboard shows
  each event's trace ID, and the trace shows each event its request recorded. A
  rolled-back transaction counts nothing. Telemetry is best effort, though: a
  request cancelled while its `COMMIT` is in flight can commit an event that is
  never logged or counted. The stored history is the audit record.
- **Failures.** A 5xx response marks its span failed and returns the trace ID in
  `X-Trace-Id`, which CORS exposes to public clients' origins. JSON error bodies
  do not repeat it. A page answering a server error, on auth or the admin
  dashboard, shows the same trace ID as a reference to quote in a report, since
  a browser never shows the header, and so does the error page a navigation's
  failure gets ([errors](request-lifecycle.md#errors)); no other page shows one. `tracing::error!` events with an `error` field, such as
  `ApiError::log_error`'s, also mark their span. Every request of an
  administrator the dashboard or auth's handoff page refuses is logged as `admin
  dashboard request refused`, with its `admin.failure` reason.
- **One instance per process.** Each process gets a new UUID as its
  `service.instance.id`, so replicas and command-line runs tell their spans and
  logs apart. The admin dashboard's requests carry `sourceorigin.listener =
  "admin"`, and their rows `listener = 'admin'`, which tells them apart from
  the public listener's.

## Adding telemetry

**A log line or span:** the `tracing` macros. Auth's own targets
(`auth_service`, `sourceorigin_repository`) are traced and logged at `info`; use
`debug!` for detail nobody needs in production.

**A count derived from an event:** a query over the `events` table, which
`Committed::publish` in `crates/database/src/telemetry.rs` fills after each
commit, so it never counts a change that rolled back. Give it a column there if
the query needs one of the event's vocabulary values.

**Anything else to count or measure:** an analytics [signal](analytics.md#signals),
recorded next to the code it measures, such as `throttled`:

```rust
Throttled { category: category.as_str() }.record();
```

**An outbound HTTP call:** `request.send_traced()` instead of `send()`. It
records the method, host and status only. `SendTraced` lives in `sourceorigin-common`
so the Rust SDK sends its requests, such as Google's signing-key fetches through
`KeyCache`, the same way.

## What never goes in telemetry

Everything a span or log line holds is stored. Never record a secret
(cookie, code, token, ceremony state, invitation, client secret), a query string,
a request header, a request path, a client IP or a user agent. Events keep IPs
and user agents under their own retention rules. User IDs are pseudonymous, so
they are allowed on spans and log lines, which are kept about 30 days. The
exporters also drop `url.path`, `url.query`, `url.full`, `client.address` and
`http.*.header.*` attributes, as a second line of defense.

Some errors quote personal data, so they are logged with care:

- **Errors** are logged as `error = %ErrorChain(&error)`: each message of the
  chain, never a type's `Debug`. `DatabaseError`'s `Debug` is that chain too, so
  even `?error` on it, or on an error holding it, leaves out PostgreSQL's
  `detail`, which quotes the rejected values (`Key (address)=(…) already
  exists`). A stored value that does not decode keeps only its column and, for
  JSON, the failure's category and position (`DecodeFailure`), never the
  decoder's message, which can quote the value.
- **SMTP failures** keep only the reply code or failure category, as the
  delivery's error. The relay's reply text can quote the recipient, so it is
  never logged, at any level.

`passkey::tests::telemetry::telemetry_exports_no_secret_of_a_sign_in_and_code_exchange`
runs a passkey sign-in and a code exchange through the production layers, with
`Telemetry::capture`, and fails if any of their secrets is exported. Extend it
when a flow handles a new kind of secret.

## Storage

The OpenTelemetry SDK's batch processors hand spans and log records to two
exporters of `crates/telemetry/src/export`, which insert each batch into
`spans` or `logs` as one `JSONEachRow` insert, through the analytics writer's
`Store`, on the service's tokio runtime. `migrate` creates and extends both
tables from their declarations, `SPANS` and `LOGS`, as it does the signals':
by day, with a bloom filter on the trace ID, so a lookup by trace reads little.
Each row struct's fields are its table's columns, which a test checks, so a
column is added to both:

| Table | Kept | One row per |
| --- | --- | --- |
| `spans` | 14 days | span: its trace, parent, name, kind, status, duration and attributes |
| `logs` | 30 days | log record exported at the service's level: its trace, level, target, message and fields |

The exporters drop `url.path`, `url.query`, `url.full`, `client.address` and
`http.*.header.*` attributes, whoever sets them, as a second line of defense:
auth never records them. Span events are not stored; the log records inside a
span carry its trace ID instead.

## Configuration

`ANALYTICS_URL` and `ANALYTICS_PASSWORD` (see
[analytics](analytics.md#the-writer)), and the standard variables the SDK reads:

| Variable | Meaning |
| --- | --- |
| `OTEL_SERVICE_NAME` | Overrides the default, `auth-service` or `sourceorigin-web` |
| `OTEL_RESOURCE_ATTRIBUTES` | Only its `service.name` and `service.instance.id` matter: the latter replaces the process's own UUID on spans and logs |
| `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG` | Sampling; every trace is kept by default |
| `RUST_LOG` | What is logged; spans keep the service's default filter. A value that does not parse stops every command |

When `ANALYTICS_URL` or `ANALYTICS_PASSWORD` is set but invalid, `serve` and the
web app refuse to start: a server must not run without the telemetry its
operators read. Every other `auth-service` command, such as `migrate` or an account
repair, logs a warning and runs with logs on stderr only, so broken telemetry
never blocks a break-glass fix. An unreachable ClickHouse never stops anything:
its batches are dropped and the SDK logs the failure on stderr.

Shutdown flushes pending telemetry after the listeners stop, for up to 3
seconds (see [Stopping](../operations/deploying.md#stopping)).

## Locally

```sh
just analytics scratch -- sh -c 'export ANALYTICS_URL=$TEST_ANALYTICS_URL ANALYTICS_PASSWORD=env://TEST_ANALYTICS_PASSWORD; just db migrate && just auth serve'
```

This runs a throwaway ClickHouse, creates its tables, then serves auth with
your own configuration, writing its spans, logs and rows there. The ClickHouse
is deleted once auth stops. See
[the analytics database](../operations/analytics.md#locally).

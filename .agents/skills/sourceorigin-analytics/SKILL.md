---
name: sourceorigin-analytics
description: >
  Read SourceOrigin's analytics database — every committed event, request,
  sign-in step, page timing, script error, span and log line — to find patterns
  and fix security, performance and reliability issues. Use when asked to
  investigate an attack, a failure, a slow page or a trace ID, to check what
  a change did in a running environment, or to look for anything unusual. Read
  only, through `auth-service analytics`, on your own preview freely; on
  staging or production only when the owner asks.
---

# Reading analytics

Each environment and preview has a ClickHouse holding what auth and
sourceorigin.dev record (see `docs/dev/architecture/analytics.md` and
`docs/dev/operations/analytics.md`). Read it with `auth-service analytics`,
which runs as `sourceorigin_reader`: ClickHouse lets it only read, and stops a
query past 5 seconds, 512 MiB or 10,000 rows. Never read it as another user,
and never with a password you found: report a refusal to the owner.

## Where you may read

- **Your own preview**, at will. Deploy it with the `preview` skill; it starts
  empty, so make the traffic you want to study, such as signing in or running
  the browser suite against it.
- **Staging or production**, only when the owner asks you to, in this
  conversation. Read; never deploy, change or delete anything there.

## Run it

```sh
branch=$(git branch --show-current)
run() { piquelctl preview exec sourceorigin "$branch" auth -- auth-service --operator "$USER-agent" --json analytics "$@"; }
run report                       # what the dashboard's Analytics page charts
run trace 4bf92f3577b34da6a3ce929d0e0e4736   # one trace's spans and log lines
run query "SELECT kind, count() FROM events GROUP BY kind ORDER BY count() DESC"
```

Add `--slot NAME` after the branch when you share a branch's preview. When the
owner asks for production, the same commands run through
`piquelctl app exec sourceorigin auth --env production -- auth-service …`.
With `--json`, the result is the one line on standard output; read only lines
starting with `{` or `[` from standard error.

`query` takes ClickHouse SQL without a `FORMAT` clause and prints each row as
a JSON object. `DESCRIBE TABLE events` names a table's columns and what each
holds; `SHOW TABLES` lists them. Rows hold no account, name, address, path or
secret: follow an event's `trace` to its spans and logs instead.

## Patterns worth a query

Times are UTC; narrow `time` first, as every table is sorted by it last.

**Security**

```sql
-- Failed sign-ins by reason and method, per hour: a spike in one is an attack.
SELECT toStartOfHour(time) AS hour, method, reason, count() FROM events
WHERE kind = 'sign_in_failed' AND time > now() - INTERVAL 1 DAY
GROUP BY ALL ORDER BY hour, count() DESC;

-- What rate limits refused, by budget: a row per refused request.
SELECT toStartOfHour(time) AS hour, category, count() FROM throttled
WHERE time > now() - INTERVAL 1 DAY GROUP BY ALL ORDER BY hour;

-- What administrators and operators did, to explain or to alarm.
SELECT attributes['event.actor'] AS actor, attributes['event.kind'] AS kind, count()
FROM logs WHERE body = 'event recorded' AND actor IN ('admin', 'operator')
  AND time > now() - INTERVAL 7 DAY GROUP BY ALL ORDER BY count() DESC;

-- Requests refused by the security layer, by status and route.
SELECT status, route, count() FROM requests
WHERE status IN (400, 401, 403, 429) AND time > now() - INTERVAL 1 DAY
GROUP BY ALL ORDER BY count() DESC LIMIT 20;
```

**Performance**

```sql
-- The slowest routes, by their 95th percentile.
SELECT method, route, count(), quantile(0.95)(duration_us) / 1000 AS p95_ms FROM requests
WHERE time > now() - INTERVAL 1 DAY GROUP BY ALL ORDER BY p95_ms DESC LIMIT 10;

-- The slowest spans inside one route: where its time goes.
SELECT name, count(), quantile(0.95)(duration_us) / 1000 AS p95_ms FROM spans
WHERE trace IN (SELECT assumeNotNull(trace) FROM requests
                WHERE route = '/login' AND time > now() - INTERVAL 1 DAY AND trace IS NOT NULL)
GROUP BY name ORDER BY p95_ms DESC LIMIT 10;

-- Pages browsers find slow, by largest contentful paint.
SELECT route, count(), quantile(0.75)(lcp_ms), quantile(0.75)(inp_ms) FROM page_timings
WHERE time > now() - INTERVAL 1 DAY GROUP BY route ORDER BY 3 DESC;

-- Database pool pressure, per replica.
SELECT toStartOfMinute(time) AS minute, instance, max(used), min(idle) FROM database_pool
WHERE time > now() - INTERVAL 1 HOUR GROUP BY ALL ORDER BY minute;
```

**Reliability**

```sql
-- Server errors, with a trace to follow for each route.
SELECT route, count(), any(trace) FROM requests
WHERE status >= 500 AND time > now() - INTERVAL 1 DAY GROUP BY route ORDER BY count() DESC;

-- Script errors browsers hit, by kind and script.
SELECT name, script, route, count() FROM script_errors
WHERE time > now() - INTERVAL 1 DAY GROUP BY ALL ORDER BY count() DESC;

-- Deliveries that fail, and the backlog they leave.
SELECT channel, outcome, count() FROM delivery_attempts
WHERE time > now() - INTERVAL 1 DAY GROUP BY ALL;
SELECT channel, max(pending), max(oldest_age_s) FROM delivery_backlogs
WHERE time > now() - INTERVAL 1 HOUR GROUP BY channel;

-- Warnings and errors auth logged, by message.
SELECT level, target, body, count() FROM logs
WHERE level IN ('ERROR', 'WARN') AND time > now() - INTERVAL 1 DAY
GROUP BY ALL ORDER BY count() DESC LIMIT 20;
```

## Follow a lead

1. A pattern names a route, a kind or a time: narrow to it, and take a `trace`.
2. `analytics trace <id>` shows what that request did, span by span, and its
   log lines.
3. Find the code from the span names and log targets, write a test that
   reproduces it, fix it, and check on your preview that the pattern changed.

Product tables (`authorization_steps`, `passkeys_used`) count only consenting
people, by day, and arrive within the hour: don't read individuals into them.

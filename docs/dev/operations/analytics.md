# Analytics database

This page helps you run each environment's analytics database, a ClickHouse
server, and read it when you need to: sign in to its pages, look up a failed
request by its trace ID, follow what administrators do, and check how much disk
it uses.

Every environment and preview runs `clickhouse`, beside `postgres`, from
`infra/piqueld.toml`. Auth and sourceorigin.dev's web server write their
[signals](../architecture/analytics.md) there, and their
[spans and logs](../architecture/telemetry.md). It needs neither auth nor the
admin dashboard to be read, so it works when they don't.
A deployment starts it before auth, so `migrate` can create its tables, but
nothing waits on it after that. When it is down, auth signs people in as usual
and drops the rows it can't send, and `migrate` warns that it could not reach
it and lets the deployment go on.

## Sign in to ClickHouse

ClickHouse serves its own pages on a private route, reachable from the
tailnet only:

| Deployment | Address |
| --- | --- |
| production | `https://analytics.sourceorigin.dev` |
| staging | `https://analytics.staging.sourceorigin.dev` |
| previews | `https://analytics-<slug>.dev.sourceorigin.dev` |

1. Print the password of ClickHouse's `sourceorigin` user:

   ```sh
   piquelctl app exec sourceorigin clickhouse --env production -- cat /run/secrets/analytics-password
   ```

   For a preview, run `piquelctl preview exec sourceorigin <branch> clickhouse -- cat /run/secrets/analytics-password`.

2. Open `/play` at the environment's address, such as
   `https://analytics.sourceorigin.dev/play`, and sign in as `sourceorigin`
   with that password.
3. Run a query, such as the last day's events, by kind:

   ```sql
   SELECT kind, count() FROM sourceorigin.events
   WHERE time > now() - INTERVAL 1 DAY
   GROUP BY kind ORDER BY count() DESC
   ```

`/dashboard` charts the server's own health. From a terminal, the same
queries run through `clickhouse-client`:

```sh
piquelctl app exec sourceorigin clickhouse --env production -- clickhouse-client --user sourceorigin --password "<password>" --database sourceorigin
```

## Look up a failed request

A trace ID names one request. You find it:

- below a server error on auth's pages and the admin dashboard, as the
  reference a user's report quotes;
- in the `X-Trace-Id` header of an API response;
- next to an event, on the admin dashboard's **Events** page.

On `/play`, read its spans in the order they started, then its log lines:

```sql
SELECT time, name, status, duration_us, attributes FROM sourceorigin.spans
WHERE trace = '<trace ID>' ORDER BY time;

SELECT time, level, body, attributes FROM sourceorigin.logs
WHERE trace = '<trace ID>' ORDER BY time
```

Spans are kept 14 days, and logs 30.

## Watch administrator activity

Every committed event is logged as `event recorded`, with its kind, the kind
of actor, the account and, from the admin dashboard, the administrator. To
notice a stolen administrator session or a compromised server, read what
administrators and operators did over the last week:

```sql
SELECT attributes['event.actor'] AS actor, attributes['event.administrator'] AS administrator,
       attributes['event.kind'] AS kind, count()
FROM sourceorigin.logs
WHERE body = 'event recorded' AND actor IN ('admin', 'operator')
  AND time > now() - INTERVAL 7 DAY
GROUP BY ALL ORDER BY count() DESC
```

and the dashboard requests auth refused, by reason:

```sql
SELECT attributes['admin.failure'] AS reason, count() FROM sourceorigin.logs
WHERE body = 'admin dashboard request refused' AND time > now() - INTERVAL 7 DAY
GROUP BY reason
```

Work you know of should explain each. For anything that isn't, look up its
traces. Log lines, like all telemetry, are best effort: a request cancelled
while its commit is in flight can record an event that is never logged (see
[telemetry](../architecture/telemetry.md)). The stored history, on the admin
dashboard's **Events** page or through `auth-service audit list`, is the audit
record. If something looks wrong, see
[a stolen administrator session](incidents.md#a-stolen-administrator-session-or-a-compromised-administrator-account).

## What it keeps

`SHOW TABLES FROM sourceorigin` lists the tables, and
`DESCRIBE TABLE sourceorigin.events` a table's columns with what each holds.
Each table keeps rows for the retention its signal declares, such as 390 days
for `events`; `spans` keeps 14 days and `logs` 30.

ClickHouse's own logs, other than `system.query_log`, which it keeps a week,
are off: `infra/clickhouse/config.xml` lists them. It logs warnings and errors
to its container's output.

## Check how much it stores

```sql
SELECT table, formatReadableSize(sum(bytes_on_disk)) AS stored, sum(rows) AS rows
FROM system.parts WHERE active AND database = 'sourceorigin'
GROUP BY table ORDER BY sum(bytes_on_disk) DESC
```

`infra/clickhouse/config.xml` compresses every column with ZSTD. ClickHouse
caps its memory at three quarters of the container's 1 GiB.

## Users and secrets

| User | Password | Used by | Allowed to |
| --- | --- | --- | --- |
| `sourceorigin` | `analytics-password` | the `migrate` job, and you on its pages | everything, and managing users |
| `sourceorigin_writer` | `analytics-writer-password` | `auth`, `web` | insert rows, into any table |

The image configures `sourceorigin` from the password it mounts each time
ClickHouse starts, so regenerating `analytics-password` takes effect on the
next deployment, which restarts `clickhouse`. `migrate` creates
`sourceorigin_writer` as a SQL user, and every deployment's `migrate` resets
its password to `analytics-writer-password`, so regenerate that one like
[the database runtime password](secrets.md#the-database-runtime-password).

## Locally

```sh
just analytics scratch -- sh -c 'export ANALYTICS_URL=$TEST_ANALYTICS_URL ANALYTICS_PASSWORD=env://TEST_ANALYTICS_PASSWORD; just db migrate && just auth serve'
```

This starts a throwaway ClickHouse in Docker, creates its tables, and serves
auth with your own configuration, writing its spans, logs and rows there. The
container is deleted once auth stops. To point a local auth at a ClickHouse of
your own, set `ANALYTICS_URL` and `ANALYTICS_PASSWORD`, and run
`just db migrate` with `ANALYTICS_OWNER_URL` and `ANALYTICS_OWNER_PASSWORD`
set to its owner. See [configuration](configuration.md).

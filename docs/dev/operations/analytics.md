# Analytics database

This page helps you run each environment's analytics database, a ClickHouse
server, and read it when you need to: sign in to its pages, query a table, and
check how much disk it uses.

Every environment and preview runs `clickhouse`, beside `postgres`, from
`infra/piqueld.toml`. Auth writes its [signals](../architecture/analytics.md) there.
Nothing else depends on it: when it is down, auth signs people in as usual and
drops the rows it can't send.

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
   SELECT kind, count() FROM events
   WHERE time > now() - INTERVAL 1 DAY
   GROUP BY kind ORDER BY count() DESC
   ```

`/dashboard` charts the server's own health. From a terminal, the same
queries run through `clickhouse-client`:

```sh
piquelctl app exec sourceorigin clickhouse --env production -- clickhouse-client --user sourceorigin --password "<password>"
```

## What it keeps

`SHOW TABLES` lists the tables, and `DESCRIBE TABLE events` a table's columns
with what each holds. Each table keeps rows for the retention its signal
declares, such as 400 days for `events`.

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
| `sourceorigin_writer` | `analytics-writer-password` | `auth` | insert rows |

The image creates `sourceorigin` on its volume's first start, with the
password it mounts then. Regenerating `analytics-password` later doesn't change
it: change it in ClickHouse first, with
`ALTER USER sourceorigin IDENTIFIED BY '<new password>'`, then store the same
value as the secret and deploy. Every deployment's `migrate` resets
`sourceorigin_writer`'s password to `analytics-writer-password`, so regenerate
that one like [the database runtime password](secrets.md#the-database-runtime-password).

## Locally

```sh
just analytics scratch -- env
```

This starts a throwaway ClickHouse in Docker and prints `TEST_ANALYTICS_URL`
among the variables it sets; the container is deleted when the command ends.
To point a local auth at a ClickHouse of your own, set `ANALYTICS_URL` and
`ANALYTICS_PASSWORD`, and run `just db migrate` with `ANALYTICS_OWNER_URL`
and `ANALYTICS_OWNER_PASSWORD` set to its owner. See
[configuration](configuration.md).

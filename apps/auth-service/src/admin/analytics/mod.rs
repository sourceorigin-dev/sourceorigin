//! The dashboard's Analytics page: what the environment's analytics database
//! counts, read as the dashboard's reader, which may only read, within limits on
//! each query. See `docs/dev/architecture/analytics.md`.

use super::{AdminError, AdminService, Operator};
use crate::charted::{DAY, HOUR, Period, StepCount, period};
use chrono::Utc;
use serde::Deserialize;
use sourceorigin_common::admin::{AuthorizationStepKind, PasskeyTransport};
use sourceorigin_telemetry::analytics::Store;

/// Requests in one hour, of every service.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
pub(crate) struct HourOfRequests {
    /// The hour's start, in seconds since the epoch.
    pub(crate) hour: i64,
    /// Every request answered.
    pub(crate) requests: u32,
    /// Those answered with a server error.
    pub(crate) errors: u32,
}

period!(HourOfRequests, hour, HOUR, 24);

/// Sign-ins in one day.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
pub(crate) struct DayOfSignIns {
    /// The day's start, in seconds since the epoch.
    pub(crate) day: i64,
    /// Browsers that signed in.
    pub(crate) signed_in: u32,
    /// Rejected passkey and provider authentications.
    pub(crate) failed: u32,
}

period!(DayOfSignIns, day, DAY, 30);

/// How long one route of one service takes.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct RouteTiming {
    /// The service that answered, such as `auth-service`.
    pub(crate) service: String,
    /// Its listener, when it has several, such as `admin`.
    pub(crate) listener: Option<String>,
    /// The request's method.
    pub(crate) method: String,
    /// The route template.
    pub(crate) route: String,
    /// Requests to it.
    pub(crate) requests: u32,
    /// The median duration, in milliseconds.
    pub(crate) median_ms: f64,
    /// The duration 95 in 100 requests finish within, in milliseconds.
    pub p95_ms: f64,
}

/// How often one kind of passkey was used.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct PasskeyKind {
    /// Whether it is synced between devices.
    pub(crate) synced: bool,
    /// How it can be reached.
    pub(crate) transport: PasskeyTransport,
    /// How often.
    pub(crate) count: u32,
}

impl PasskeyKind {
    /// `rows`, one per kind, most used first: spellings this release doesn't
    /// know read as one transport, so their counts add up.
    fn merged(rows: Vec<Self>) -> Vec<Self> {
        let mut kinds: Vec<Self> = Vec::with_capacity(rows.len());
        for row in rows {
            match kinds
                .iter_mut()
                .find(|kind| (kind.synced, kind.transport) == (row.synced, row.transport))
            {
                Some(kind) => kind.count = kind.count.saturating_add(row.count),
                None => kinds.push(row),
            }
        }
        kinds.sort_by_key(|kind| std::cmp::Reverse(kind.count));
        kinds
    }
}

/// What one table stores.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct TableSize {
    /// Its name.
    pub(crate) table: String,
    /// Its rows.
    pub(crate) rows: f64,
    /// The bytes it takes on disk, compressed.
    pub(crate) bytes: f64,
}

/// What the Analytics page shows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Analytics {
    /// Requests per hour, every one of the last 24, oldest first.
    pub(crate) requests: Vec<HourOfRequests>,
    /// The slowest routes of those hours, slowest first.
    pub(crate) slowest: Vec<RouteTiming>,
    /// Sign-ins per day, every one of the last 30, oldest first.
    pub(crate) sign_ins: Vec<DayOfSignIns>,
    /// How often each step of signing in to an app was reached in those days,
    /// in the order a sign-in passes them.
    pub(crate) steps: Vec<(AuthorizationStepKind, u32)>,
    /// The kinds of passkeys used in those days, most used first.
    pub(crate) passkeys: Vec<PasskeyKind>,
    /// What each table stores, largest first.
    pub(crate) tables: Vec<TableSize>,
}

impl AdminService {
    /// Reads the Analytics page through `reader`, ClickHouse's read-only user,
    /// running its queries at once.
    ///
    /// # Errors
    /// Fails when the operator may not read, or ClickHouse refuses or doesn't
    /// answer a query.
    pub(crate) async fn analytics(
        &self,
        operator: &Operator,
        reader: &Store,
    ) -> Result<Analytics, AdminError> {
        operator.authorize_read(&self.database).await?;
        let now = Utc::now();
        let (hour, day) = (HourOfRequests::current(now), DayOfSignIns::current(now));
        let hours = HourOfRequests::since(hour).to_string();
        let days = DayOfSignIns::since(day).to_string();
        let (hours, days) = (&[("since", hours.as_str())], &[("since", days.as_str())]);
        let (requests, slowest, sign_ins, steps, passkeys, tables) = tokio::try_join!(
            reader.query(
                "SELECT toUnixTimestamp(toStartOfHour(time, 'UTC')) AS hour,
                    toUInt32(count()) AS requests, toUInt32(countIf(status >= 500)) AS errors
                FROM requests WHERE time >= toDateTime({since:Int64}, 'UTC')
                GROUP BY hour ORDER BY hour",
                hours,
            ),
            reader.query(
                "SELECT service, listener, method, route,
                    toUInt32(count()) AS requests,
                    quantile(0.5)(duration_us) / 1000 AS median_ms,
                    quantile(0.95)(duration_us) / 1000 AS p95_ms
                FROM requests WHERE time >= toDateTime({since:Int64}, 'UTC') AND route IS NOT NULL
                GROUP BY service, listener, method, route ORDER BY p95_ms DESC LIMIT 8",
                hours,
            ),
            reader.query(
                "SELECT toUnixTimestamp(toStartOfDay(time, 'UTC')) AS day,
                    toUInt32(countIf(kind = 'signed_in')) AS signed_in,
                    toUInt32(countIf(kind = 'sign_in_failed')) AS failed
                FROM events WHERE time >= toDateTime({since:Int64}, 'UTC')
                    AND kind IN ('signed_in', 'sign_in_failed')
                GROUP BY day ORDER BY day",
                days,
            ),
            reader.query(
                "SELECT step, toUInt32(count()) AS count FROM authorization_steps
                WHERE time >= toDateTime({since:Int64}, 'UTC') GROUP BY step",
                days,
            ),
            reader.query(
                "SELECT synced, transport, toUInt32(count()) AS count FROM passkeys_used
                WHERE time >= toDateTime({since:Int64}, 'UTC')
                GROUP BY synced, transport ORDER BY count DESC",
                days,
            ),
            reader.query(
                "SELECT table, toFloat64(sum(rows)) AS rows, toFloat64(sum(bytes_on_disk)) AS bytes
                FROM system.parts WHERE active AND database = currentDatabase()
                GROUP BY table ORDER BY bytes DESC",
                &[],
            ),
        )?;
        Ok(Analytics {
            requests: HourOfRequests::every(requests, hour),
            slowest,
            sign_ins: DayOfSignIns::every(sign_ins, day),
            steps: StepCount::funnel(&steps),
            passkeys: PasskeyKind::merged(passkeys),
            tables,
        })
    }
}

#[cfg(test)]
mod tests;

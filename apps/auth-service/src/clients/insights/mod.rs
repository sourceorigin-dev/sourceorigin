//! What an app's owner sees of how it is used, on its Insights tab. It is read
//! only through an [`OwnedClient`], which only the owner holds, and only of that
//! app, over whole days, so it changes once a day. What people did on auth's
//! pages shows [`Floored`] by [`Noise`]: each day's counts moved by that day's
//! secret amount, rounded, then added up.

use super::OwnedClient;
use crate::charted::{DAY, Period, StepCount, period};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sourceorigin_common::{ClientId, admin::AuthorizationStepKind, protocol::oauth::GrantType};
use sourceorigin_crypto::{Digest, Secret};
use sourceorigin_telemetry::analytics::{PRODUCT_INTERVAL, Store, StoreError};

/// What each day's count of steps rounds to a multiple of, so a day's few
/// steps, which could single out a person to an app that knows its few users,
/// don't show exactly. They count steps, not people: one person may reach a
/// step twice. The page, the user guide and the privacy policy name it.
pub(crate) const FLOOR: u32 = 5;

/// A count of steps people reached on auth's pages, as an app's owner sees it:
/// the sum of each day's, moved and rounded down by [`Noise`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Floored {
    /// Every day rounded down to none.
    Few,
    /// About this many, a multiple of [`FLOOR`].
    About(u32),
}

impl Floored {
    /// `total`, a sum of rounded days, so a multiple of [`FLOOR`], as its
    /// owner sees it. Only [`Noise::floored`] makes one.
    const fn of(total: u32) -> Self {
        if total == 0 {
            Self::Few
        } else {
            Self::About(total)
        }
    }
}

/// One day's count of one step of signing in to an app.
#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct DayOfStep {
    /// The day's start, in seconds since the epoch.
    pub(crate) day: i64,
    /// The step.
    pub(crate) step: AuthorizationStepKind,
    /// How often.
    pub(crate) count: u32,
}

/// What moves each day's counts of steps an app's owner sees: a secret amount
/// under [`FLOOR`] for each app and day counted, the same for every step of
/// that day, on every replica and every reload, so a day draws once for good
/// and a step it reached less often never shows more than one it reached more
/// often. Then each count is rounded down to a
/// multiple of [`FLOOR`], which keeps their sums about right. A lone sign-in,
/// all of its steps at once, shows one time in [`FLOOR`]; see
/// `docs/dev/architecture/analytics.md` for what remains.
#[derive(Clone)]
pub(crate) struct Noise(Secret);

impl Noise {
    /// Moves counts by what `key`, derived from the token digest key, says.
    pub(crate) const fn new(key: Secret) -> Self {
        Self(key)
    }

    /// The amount `client`'s counts of the day starting at `day` move by:
    /// evenly one of `0..FLOOR`.
    pub(crate) fn offset(&self, client: &ClientId, day: i64) -> u32 {
        let value = format!("{client}\n{day}");
        let digest = Digest::hmac(self.0.as_bytes(), value.as_bytes());
        let mut bytes = [0; 4];
        bytes.copy_from_slice(&digest.as_bytes()[..4]);
        u32::from_be_bytes(bytes) % FLOOR
    }

    /// How far sign-ins to `client` got, of `days`, as its owner sees them:
    /// each day moved and rounded down, then added up per step, in the order a
    /// sign-in passes them, and a step another release wrote only once it
    /// shows, so its mere presence says nothing.
    pub(crate) fn floored(
        &self,
        client: &ClientId,
        days: &[DayOfStep],
    ) -> Vec<(AuthorizationStepKind, Floored)> {
        let rounded: Vec<StepCount> = days
            .iter()
            .map(|day| {
                let moved = day.count.saturating_add(self.offset(client, day.day));
                StepCount {
                    step: day.step,
                    count: moved - moved % FLOOR,
                }
            })
            .collect();
        StepCount::funnel(&rounded)
            .into_iter()
            .map(|(step, total)| (step, Floored::of(total)))
            .collect()
    }
}

/// The tokens an app was issued in one day: what the app itself receives, so
/// counted exactly.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
pub(crate) struct DayOfTokens {
    /// The day's start, in seconds since the epoch.
    pub(crate) day: i64,
    /// Those exchanging a sign-in's code.
    pub(crate) exchanged: u32,
    /// Those refreshing a sign-in.
    pub(crate) refreshed: u32,
}

period!(DayOfTokens, day, DAY, 30);

/// What an app's Insights tab shows, over its last 30 whole days.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppInsights {
    /// How far sign-ins to it got: each step a sign-in passes, in order, and
    /// one another release counts, if it shows.
    pub(crate) steps: Vec<(AuthorizationStepKind, Floored)>,
    /// The tokens it was issued per day, every one of the 30, oldest first.
    pub(crate) tokens: Vec<DayOfTokens>,
}

impl AppInsights {
    /// The start of the day Insights read up to at `now`, in seconds since
    /// the epoch: the end of the newest whole day whose product rows have all
    /// been sent. Replicas send them every [`PRODUCT_INTERVAL`], so a day's
    /// counts show once twice that has passed since it ended, all at once.
    pub(crate) fn until(now: DateTime<Utc>) -> i64 {
        DayOfTokens::current(now - PRODUCT_INTERVAL * 2)
    }
}

impl OwnedClient {
    /// This app's Insights, read through `analytics`, ClickHouse's read-only
    /// user, up to [`AppInsights::until`], with its counts of steps
    /// [`Noise::floored`].
    ///
    /// # Errors
    /// Fails when ClickHouse refuses or doesn't answer a query.
    pub(crate) async fn insights(
        &self,
        analytics: &Store,
        noise: &Noise,
    ) -> Result<AppInsights, StoreError> {
        let until = AppInsights::until(Utc::now());
        let newest = until - DAY;
        let client = &self.client().id;
        let (since, end) = (DayOfTokens::since(newest).to_string(), until.to_string());
        let parameters = &[
            ("client", client.as_str()),
            ("since", &since),
            ("until", &end),
            ("exchange", GrantType::AuthorizationCode.as_str()),
            ("refresh", GrantType::RefreshToken.as_str()),
        ];
        let (steps, tokens) = tokio::try_join!(
            analytics.query::<DayOfStep>(
                "SELECT toUnixTimestamp(toStartOfDay(time, 'UTC')) AS day, step,
                    toUInt32(count()) AS count
                FROM authorization_steps
                WHERE client = {client:String} AND time >= toDateTime({since:Int64}, 'UTC')
                    AND time < toDateTime({until:Int64}, 'UTC')
                GROUP BY day, step",
                parameters,
            ),
            analytics.query(
                "SELECT toUnixTimestamp(toStartOfDay(time, 'UTC')) AS day,
                    toUInt32(countIf(grant_type = {exchange:String})) AS exchanged,
                    toUInt32(countIf(grant_type = {refresh:String})) AS refreshed
                FROM tokens_issued
                WHERE client = {client:String} AND time >= toDateTime({since:Int64}, 'UTC')
                    AND time < toDateTime({until:Int64}, 'UTC')
                GROUP BY day ORDER BY day",
                parameters,
            ),
        )?;
        Ok(AppInsights {
            steps: noise.floored(client, &steps),
            tokens: DayOfTokens::every(tokens, newest),
        })
    }
}

#[cfg(test)]
mod tests;

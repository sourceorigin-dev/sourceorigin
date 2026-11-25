//! What the charts of analytics read back, for the dashboard's Analytics page
//! and each app's Insights: hours and days, every one of them, whether they
//! counted anything or not, and how often each step of signing in was reached.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use sourceorigin_common::admin::AuthorizationStepKind;

/// An hour, in seconds.
pub(crate) const HOUR: i64 = 60 * 60;
/// A day, in seconds.
pub(crate) const DAY: i64 = 24 * HOUR;

/// One period of a chart over time, which the page shows every one of, oldest
/// first, up to the newest it shows, whether it counted anything or not.
pub(crate) trait Period: Sized {
    /// Its length, in seconds.
    const SECONDS: i64;
    /// How many the page shows.
    const COUNT: i64;

    /// Its start, in seconds since the epoch.
    fn start(&self) -> i64;

    /// The period starting at `start`, which counted nothing.
    fn empty(start: i64) -> Self;

    /// The start of the oldest period shown up to the one starting at
    /// `newest`, in seconds since the epoch: what the queries read from.
    fn since(newest: i64) -> i64 {
        newest - (Self::COUNT - 1) * Self::SECONDS
    }

    /// The start of `now`'s period, in UTC.
    fn current(now: DateTime<Utc>) -> i64 {
        now.timestamp().div_euclid(Self::SECONDS) * Self::SECONDS
    }

    /// One row per period shown up to the one starting at `newest`, taking
    /// each from `rows`, which come sorted by start, or an empty one.
    fn every(rows: Vec<Self>, newest: i64) -> Vec<Self> {
        let mut rows = rows.into_iter().peekable();
        (0..Self::COUNT)
            .map(|n| Self::since(newest) + n * Self::SECONDS)
            .map(|at| {
                while rows.next_if(|row| row.start() < at).is_some() {}
                rows.next_if(|row| row.start() == at)
                    .unwrap_or_else(|| Self::empty(at))
            })
            .collect()
    }

    /// Its start, written in UTC as `format` says, such as `%b %-d`.
    fn label(&self, format: &str) -> String {
        DateTime::from_timestamp(self.start(), 0)
            .map_or_else(String::new, |at| at.format(format).to_string())
    }
}

/// Makes `$name` a [`Period`] `$seconds` long, `$count` of them, starting at its
/// `$start` field; when it counted nothing, its other fields are their
/// defaults.
macro_rules! period {
    ($name:ty, $start:ident, $seconds:expr, $count:expr) => {
        impl $crate::charted::Period for $name {
            const SECONDS: i64 = $seconds;
            const COUNT: i64 = $count;

            fn start(&self) -> i64 {
                self.$start
            }

            fn empty($start: i64) -> Self {
                Self {
                    $start,
                    ..Self::default()
                }
            }
        }
    };
}
pub(crate) use period;

/// How often one step of signing in to an app was reached, as
/// `authorization_steps` counts it.
#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct StepCount {
    /// The step.
    pub(crate) step: AuthorizationStepKind,
    /// How often.
    pub(crate) count: u32,
}

impl StepCount {
    /// How often each step was reached, of `rows`, in the order a sign-in
    /// passes them, and steps another release wrote, which read as one, only
    /// when counted.
    pub(crate) fn funnel(rows: &[Self]) -> Vec<(AuthorizationStepKind, u32)> {
        AuthorizationStepKind::ALL
            .into_iter()
            .map(|step| {
                let rows = rows.iter().filter(|row| row.step == step);
                (
                    step,
                    rows.fold(0_u32, |sum, row| sum.saturating_add(row.count)),
                )
            })
            .filter(|(step, count)| *step != AuthorizationStepKind::Other || *count > 0)
            .collect()
    }
}

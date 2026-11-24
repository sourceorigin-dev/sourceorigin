//! Verifies the chain on a schedule, so the dashboard's Audit tab shows a recent
//! verification without an operator asking for one.

use super::{AuditChain, AuditError, AuditKeys, AuditVerification};
use crate::jobs::{ExclusiveJob, JobCounted, Shutdown, Streak};
use chrono::{TimeDelta, Utc};
use sourceorigin_common::{
    alerts::Alert,
    events::{DomainEvent, EventContext, JobName},
};
use sourceorigin_repository::{ExclusiveConnection, LockKey, RecordedEvent, Verifications};
use sourceorigin_telemetry::analytics::Signal;
use std::time::Duration;

/// Verifies the whole chain, on one replica at a time, once the newest recorded
/// verification, whoever made it, other than one with keys given with `--key`,
/// is [`Self::VERIFY_EVERY`] old, and records what it found as
/// [`JobName::AuditVerification`]. Runs that keep failing raise the
/// [`Alert::VerificationFailing`] alert, which such a recorded verification
/// clears.
pub(crate) struct Verifier {
    /// The keys it verifies with, the dashboard's (see [`AuditKeys::for_server`]).
    keys: AuditKeys,
    /// Runs in a row on this replica that failed.
    failures: Streak,
}

impl Verifier {
    /// How old the newest recorded verification may get.
    const VERIFY_EVERY: TimeDelta = TimeDelta::hours(6);
    /// Failed runs in a row that raise [`Alert::VerificationFailing`]: a single
    /// one may be the database's passing failure.
    const FAILING_AFTER: u32 = 3;

    /// A verifier checking checkpoints against `keys`.
    pub(crate) const fn new(keys: AuditKeys) -> Self {
        Self {
            keys,
            failures: Streak::new(Self::FAILING_AFTER),
        }
    }

    /// Counts a run that ended with `ran`, raising the alert of a verification
    /// that keeps failing once enough fail in a row, and returns how it ended.
    pub(super) async fn ended<T>(
        &self,
        connection: &mut ExclusiveConnection,
        ran: Result<T, AuditError>,
    ) -> Result<T, AuditError> {
        if ran.is_ok() {
            self.failures.reset();
        } else {
            let alert = Alert::VerificationFailing {};
            self.failures
                .failed(connection, JobName::AuditVerification, alert)
                .await;
        }
        ran
    }

    /// Verifies and records the chain, unless a verification was recorded in the
    /// last [`Self::VERIFY_EVERY`], and keeps whether the newest one found it
    /// broken. Returns what it found, if it verified.
    pub(super) async fn pass(
        &self,
        connection: &mut ExclusiveConnection,
    ) -> Result<Option<AuditVerification>, AuditError> {
        let newest = connection
            .events()
            .newest_audit_verification(Verifications::OwnKeys)
            .await?;
        if let Some(newest) = newest
            && Utc::now() - newest.occurred_at < Self::VERIFY_EVERY
        {
            if let RecordedEvent::Known(DomainEvent::AuditChainVerified(verification)) =
                newest.event
            {
                Self::count(&verification);
            }
            return Ok(None);
        }
        let verification = AuditChain::verify_in(connection.begin().await?, &self.keys).await?;
        let context = EventContext::system(JobName::AuditVerification);
        AuditChain::record(connection.begin().await?, &context, &verification).await?;
        Self::count(&verification);
        Ok(Some(verification))
    }

    /// Records whether `verification`, the newest, found the chain broken.
    fn count(verification: &AuditVerification) {
        JobCounted {
            count: "audit_broken",
            value: u64::from(verification.broken.is_some()),
        }
        .record();
    }
}

impl ExclusiveJob for Verifier {
    const NAME: &'static str = "audit-verifier";
    const LOCK: LockKey = LockKey::AuditVerification;
    const EVERY: Duration = Duration::from_mins(10);
    type Error = AuditError;

    /// One verification reads one snapshot, so shutdown cancels it rather than
    /// stopping it early.
    async fn run(
        &self,
        connection: &mut ExclusiveConnection,
        _: &Shutdown,
    ) -> Result<(), AuditError> {
        let ran = self.pass(connection).await;
        self.ended(connection, ran).await.map(drop)
    }
}

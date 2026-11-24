//! What the database reports to telemetry: every committed event, as a log line
//! and an analytics row, and samples of the pool's connections.

use crate::Database;
use sourceorigin_common::{
    EventId, TraceId, UserId,
    events::{DomainEvent, HistoryActor, SignInFailure},
};
use sourceorigin_telemetry::{
    Telemetry,
    analytics::{Instance, Signal},
};
use std::time::Duration;

sourceorigin_telemetry::signal! {
    /// A domain event committed, as the audit history records it, without which
    /// account it concerns or which administrator caused it: the history keeps
    /// both, and its export and purge cover them.
    pub struct EventRecorded in "events", kept 390 days, ordered by (kind) {
        /// What happened, such as `signed_in`.
        pub kind: &'static str,
        /// The kind of actor that caused it, such as `user` or `operator`.
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

sourceorigin_telemetry::signal! {
    /// The connections a replica's database pool held, sampled every 15 seconds.
    pub struct PoolSampled in "database_pool", kept 30 days, ordered by (instance) {
        /// The replica.
        pub instance: Instance,
        /// Connections waiting for a query.
        pub idle: u32,
        /// Connections running one.
        pub used: u32,
    }
}

/// How often [`Database::sample_connections`] records a [`PoolSampled`].
const POOL_SAMPLE_EVERY: Duration = Duration::from_secs(15);

/// An event recorded in a transaction, published once the transaction commits.
pub(crate) struct Committed {
    /// The stored event's ID.
    pub(crate) id: EventId,
    /// The kind of actor that caused it.
    pub(crate) actor: HistoryActor,
    /// The administrator that caused it, if one did from the admin dashboard: the
    /// only actor whose account is not the subject, so their actions can be
    /// followed in telemetry. Operators are never named.
    pub(crate) administrator: Option<UserId>,
    /// The account it concerns.
    pub(crate) subject: Option<UserId>,
    /// What happened.
    pub(crate) event: DomainEvent,
}

impl Committed {
    /// Logs the event in the current trace, counts it and records it as an
    /// [`EventRecorded`] row. It runs only after the
    /// commit, so a rolled-back change is never counted; a commit whose answer is
    /// lost may go uncounted, so the stored history stays the record. The line names the
    /// event, its kind, the kind of actor and the pseudonymous IDs of the subject and
    /// administrator: the rest, personal data included, stays in the database under
    /// the event's ID.
    pub(crate) fn publish(&self) {
        let kind = self.event.kind();
        tracing::info!(
            event.id = %self.id,
            event.kind = kind,
            event.actor = self.actor.as_str(),
            event.subject = self.subject.map(tracing::field::display),
            event.administrator = self.administrator.map(tracing::field::display),
            "event recorded"
        );
        let (method, reason) = match &self.event {
            DomainEvent::SignedIn { method, .. } | DomainEvent::AccountCreated { method, .. } => {
                (Some(method.as_str()), None)
            }
            DomainEvent::SignInFailed { method, reason } => {
                (Some(method.as_str()), Some(SignInFailure::as_str(*reason)))
            }
            DomainEvent::GrantRevoked { reason, .. } => (None, Some(reason.as_str())),
            DomainEvent::CodeRedemptionFailed { reason, .. } => (None, Some(reason.as_str())),
            _ => (None, None),
        };
        EventRecorded {
            kind,
            actor: self.actor.as_str(),
            method,
            reason,
            event: self.id,
            trace: Telemetry::current_trace_id(),
        }
        .record();
    }
}

impl Database {
    /// Records the pool's idle and used connections as a [`PoolSampled`] every
    /// 15 seconds, on a task that ends with the runtime.
    pub fn sample_connections(&self) {
        let pool = self.pool.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(POOL_SAMPLE_EVERY);
            loop {
                every.tick().await;
                let idle = u32::try_from(pool.num_idle()).unwrap_or(u32::MAX);
                PoolSampled {
                    instance: Instance::current(),
                    idle,
                    used: pool.size().saturating_sub(idle),
                }
                .record();
            }
        });
    }
}

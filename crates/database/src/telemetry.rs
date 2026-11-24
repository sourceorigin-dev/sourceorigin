//! What the database reports to telemetry: every committed event, as a log line,
//! in counters and as an analytics row, and the pool's connections.

use crate::Database;
use sourceorigin_common::{
    EventId, TraceId, UserId,
    events::{DomainEvent, HistoryActor, SignInFailure},
};
use sourceorigin_telemetry::{Counter, Gauge, Label, Telemetry, analytics::Signal};

sourceorigin_telemetry::signal! {
    /// A domain event committed, as the audit history records it, without which
    /// account it concerns or which administrator caused it: the history keeps
    /// both, and its export and purge cover them.
    pub struct EventRecorded in "events", kept 390 days, ordered by (kind) {
        /// What happened, such as `signed_in`.
        pub kind: &'static str,
        /// The kind of actor that caused it, such as `user` or `operator`.
        pub actor: &'static str,
        /// The event's ID in the audit history.
        pub event: EventId,
        /// The trace of the request or job that recorded it.
        pub trace: Option<TraceId>,
    }
}

/// Every committed event, by kind.
static EVENTS: Counter<[Label; 1]> = Counter::new(
    "sourceorigin.auth.events",
    "{event}",
    "Domain events committed, by kind.",
);
/// Sign-ins and rejected authentications, by method and outcome.
static SIGN_INS: Counter<[Label; 2]> = Counter::new(
    "sourceorigin.auth.sign_ins",
    "{sign_in}",
    "Sign-ins by method, and their outcome: success or why they were rejected.",
);
/// New accounts, by how the person proved who they are.
static REGISTRATIONS: Counter<[Label; 1]> = Counter::new(
    "sourceorigin.auth.registrations",
    "{account}",
    "Accounts created, by authentication method.",
);
/// Revoked refresh families, by reason: replays and policy rejections included.
static GRANTS_REVOKED: Counter<[Label; 1]> = Counter::new(
    "sourceorigin.auth.grants.revoked",
    "{grant}",
    "Refresh token families revoked, by reason.",
);
/// Failed authorization code redemptions, each of which used its code up, by
/// reason.
static CODE_REDEMPTIONS_FAILED: Counter<[Label; 1]> = Counter::new(
    "sourceorigin.auth.codes.redemption_failed",
    "{code}",
    "Authorization code redemptions that failed and used the code up, by reason.",
);
/// The pool's connections, as the database semantic conventions name them.
static CONNECTIONS: Gauge<[Label; 1]> = Gauge::new(
    "db.client.connection.count",
    "{connection}",
    "Connections in the database pool, by state.",
);

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
        EVENTS.add(1, &[("kind", kind)]);
        EventRecorded {
            kind,
            actor: self.actor.as_str(),
            event: self.id,
            trace: Telemetry::current_trace_id(),
        }
        .record();
        match &self.event {
            DomainEvent::SignedIn { method, .. } => {
                SIGN_INS.add(1, &[("method", method.as_str()), ("outcome", "success")]);
            }
            DomainEvent::SignInFailed { method, reason } => {
                SIGN_INS.add(
                    1,
                    &[
                        ("method", method.as_str()),
                        ("outcome", SignInFailure::as_str(*reason)),
                    ],
                );
            }
            DomainEvent::AccountCreated { method, .. } => {
                REGISTRATIONS.add(1, &[("method", method.as_str())]);
            }
            DomainEvent::GrantRevoked { reason, .. } => {
                GRANTS_REVOKED.add(1, &[("reason", reason.as_str())]);
            }
            DomainEvent::CodeRedemptionFailed { reason, .. } => {
                CODE_REDEMPTIONS_FAILED.add(1, &[("reason", reason.as_str())]);
            }
            _ => {}
        }
    }
}

impl Database {
    /// Reports the pool's idle and used connections as `db.client.connection.count`
    /// at every metrics collection. Only the first call registers.
    pub fn observe_connections(&self) {
        let pool = self.pool.clone();
        CONNECTIONS.observe(move || {
            let idle = u64::try_from(pool.num_idle()).unwrap_or(u64::MAX);
            let used = u64::from(pool.size()).saturating_sub(idle);
            [
                (idle, [("db.client.connection.state", "idle")]),
                (used, [("db.client.connection.state", "used")]),
            ]
        });
    }
}

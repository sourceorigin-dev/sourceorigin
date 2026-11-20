//! Who caused an event, where their request came from and why, recorded beside it.

use crate::{ClientId, SessionId, UserId, user_agent::UserAgentFamily};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Who caused an event. Every feature attributes events with this one type.
///
/// Operator attribution (`os_user`, `host`, an administrator's user ID) is
/// accountability data, so it stays in the event rather than in personal data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Actor {
    /// A signed-in user, or one signing in.
    User {
        /// The acting account.
        user_id: UserId,
        /// The session acting, once there is one.
        session_id: Option<SessionId>,
    },
    /// An administrator, through the admin dashboard.
    Admin {
        /// The administrator's account.
        user_id: UserId,
        /// The administrator's session.
        session_id: SessionId,
    },
    /// An operator, through the service's command line.
    Cli {
        /// Who the operator is: the name `--operator` or `AUTH_OPERATOR` gave, or
        /// else the operating system user, `$USER`. Named `os_user` in the events
        /// already sealed.
        os_user: String,
        /// The machine's host name.
        host: String,
    },
    /// A client calling a back-channel endpoint.
    Client {
        /// The authenticated client.
        client_id: ClientId,
    },
    /// A background job.
    System {
        /// The job.
        job: JobName,
    },
    /// An unauthenticated browser, e.g. one failing to sign in.
    Anonymous,
}

impl Actor {
    /// The client acting, if any.
    #[must_use]
    pub const fn client_id(&self) -> Option<&ClientId> {
        match self {
            Self::Client { client_id } => Some(client_id),
            Self::User { .. }
            | Self::Admin { .. }
            | Self::Cli { .. }
            | Self::System { .. }
            | Self::Anonymous => None,
        }
    }

    /// The session acting, if any.
    #[must_use]
    pub const fn session_id(&self) -> Option<SessionId> {
        match self {
            Self::User { session_id, .. } => *session_id,
            Self::Admin { session_id, .. } => Some(*session_id),
            Self::Cli { .. } | Self::Client { .. } | Self::System { .. } | Self::Anonymous => None,
        }
    }
}

/// Background jobs that act on their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobName {
    /// The periodic expired-state cleanup.
    Cleanup,
    /// The purge of accounts whose deletion grace period ended.
    AccountPurge,
    /// Signing key rotation, which records the steps it takes on its own.
    KeyRotation,
    /// The periodic verification of the audit chain, which records what it found
    /// and the alert of a broken chain.
    AuditVerification,
    /// The sealer of the audit chain, which records the alert of a checkpoint it
    /// refuses to sign over events that no longer match.
    AuditSealer,
}

/// Where a request came from. Both fields are personal data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequestOrigin {
    /// The client's IP address, when the event came from a request.
    pub client_ip: Option<IpAddr>,
    /// The client's browser and platform, when the event came from a request.
    pub user_agent_family: Option<UserAgentFamily>,
}

/// Who caused an event, where their request came from and why, recorded with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventContext {
    /// Who caused the event.
    pub actor: Actor,
    /// Where the request came from; empty outside requests.
    pub origin: RequestOrigin,
    /// Why an administrator or operator changed someone's account, as they wrote
    /// it. Kept with the origin as personal data, since free text may name people,
    /// and shown to the account's owner in their security history.
    pub reason: Option<String>,
}

impl EventContext {
    /// What background `job` does on its own, outside any request.
    #[must_use]
    pub fn system(job: JobName) -> Self {
        Self {
            actor: Actor::System { job },
            origin: RequestOrigin::default(),
            reason: None,
        }
    }
}

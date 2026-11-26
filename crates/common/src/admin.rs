//! The views of administration: what the admin dashboard shows and the CLI's `--json` prints,
//! and the searches the service takes, such as [`EventSearch`](crate::admin::EventSearch).
//!
//! The dashboard's forms are not here, except two page queries, [`UserSearch`](crate::admin::UserSearch)
//! and [`SendAnnouncement`](crate::admin::SendAnnouncement), until they move to `sourceorigin-ui`.
//! Bodies the account settings also return come from [`crate::protocol::api`] and are
//! re-exported here, as is the [`UserAgentFamily`](crate::admin::UserAgentFamily) the session
//! and device views show.

pub use crate::protocol::api::{Identity, Passkey};
pub use crate::user_agent::{Browser, Platform, UserAgentFamily};
use crate::{
    AccountStatus, AnnouncementId, AuthenticationMethod, ClientId, ClientSecretId, DeliveryId,
    EventId, Handle, PasskeyId, PictureId, RecoveryId, SessionId, TokenId, TraceId, UserId,
    clients::{ClientKind, ClientStatus, RelatedOrigin},
    protocol::{
        jwt::KeyId,
        oauth::{ClientAuthMethod, GrantType},
    },
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use std::{net::IpAddr, num::NonZeroU32};
use url::Url;

/// An account, as search results and its detail page show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// Account ID.
    pub id: UserId,
    /// Public identifier and login name.
    pub handle: Handle,
    /// Name shown to the user and to applications.
    pub display_name: String,
    /// Whether the account can sign in.
    pub status: AccountStatus,
    /// The account's profile picture, served at `/pictures/{id}` on auth, while the
    /// account is active: auth serves it only then.
    pub picture: Option<PictureId>,
    /// When the account was created.
    pub created_at: DateTime<Utc>,
}

/// The account search's query.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSearch {
    /// An account ID, or part of a handle, display name or email, ignoring case.
    /// Empty matches every account.
    #[serde(default)]
    pub q: String,
    /// The page wanted, counting from 1; the first when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

/// One page of accounts, by ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserPage {
    /// The matching accounts.
    pub users: Vec<User>,
    /// Where the page stands among every match.
    pub paging: Paging,
}

/// Where a page of a list stands: pages count from 1, and a list with no match
/// has one empty page. Neither the page nor the page size is ever zero: build it
/// with [`Paging::new`], and decoding refuses a zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paging {
    /// The page shown, from 1.
    #[serde(deserialize_with = "Paging::non_zero")]
    pub page: u32,
    /// How many pages there are.
    pub pages: u32,
    /// How many items match, over every page.
    pub total: u64,
    /// How many items a page holds at most, at least 1.
    #[serde(deserialize_with = "Paging::non_zero")]
    pub per_page: u32,
}

impl Paging {
    /// Page `page` of `total` items, `per_page` at a time. A page past the last
    /// reads as the last, so a stale link still shows something, and a page size
    /// of 0 reads as 1.
    #[must_use]
    pub fn new(page: Option<u32>, total: u64, per_page: u32) -> Self {
        let per_page = per_page.max(1);
        let pages = total
            .div_ceil(u64::from(per_page))
            .clamp(1, u64::from(u32::MAX));
        let pages = u32::try_from(pages).unwrap_or(u32::MAX);
        Self {
            page: page.unwrap_or(1).clamp(1, pages),
            pages,
            total,
            per_page,
        }
    }

    /// How many items the pages before this one hold: the query's offset.
    #[must_use]
    pub fn offset(self) -> u64 {
        u64::from(self.page.saturating_sub(1)) * u64::from(self.per_page)
    }

    /// The numbers of the first and last item of the page, counting from 1, or
    /// `None` when nothing matches.
    #[must_use]
    pub fn range(self) -> Option<(u64, u64)> {
        (self.total > 0).then(|| {
            let first = self.offset() + 1;
            (
                first,
                (first + u64::from(self.per_page) - 1).min(self.total),
            )
        })
    }

    /// The page before this one, if there is one.
    #[must_use]
    pub const fn previous(self) -> Option<u32> {
        if self.page > 1 {
            Some(self.page - 1)
        } else {
            None
        }
    }

    /// The page after this one, if there is one.
    #[must_use]
    pub const fn next(self) -> Option<u32> {
        if self.page < self.pages {
            Some(self.page + 1)
        } else {
            None
        }
    }

    /// Decodes a page or a page size, refusing zero as [`Self::new`] never builds
    /// it, so a decoded page's offset can't underflow.
    fn non_zero<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
        NonZeroU32::deserialize(deserializer).map(NonZeroU32::get)
    }
}

/// Everything an administrator sees about one account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDetail {
    /// Profile and status.
    pub user: User,
    /// Whether the account is an administrator. The dashboard changes only the
    /// accounts of non-administrators.
    pub administrator: bool,
    /// Email addresses, primary first.
    pub emails: Vec<Email>,
    /// Active passkeys, without their cryptographic contents.
    pub passkeys: Vec<Passkey>,
    /// Linked social identities.
    pub identities: Vec<Identity>,
    /// Active sessions, newest first.
    pub sessions: Vec<Session>,
    /// The browsers it knows, most recently seen first.
    pub known_devices: Vec<KnownDevice>,
    /// Remembered consents to applications.
    pub consents: Vec<Consent>,
    /// The account's deletion, while it is scheduled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletion: Option<ScheduledDeletion>,
}

/// An account's scheduled deletion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledDeletion {
    /// When it was scheduled.
    pub requested_at: DateTime<Utc>,
    /// When the purge job may delete the account.
    pub purge_after: DateTime<Utc>,
    /// Why its purge keeps failing, once it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<PurgeFailure>,
}

/// Why an account's purge keeps failing, and when it is tried again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurgeFailure {
    /// Failed attempts in a row.
    pub attempts: u64,
    /// The latest failure's message chain.
    pub last_error: String,
    /// When the purge job tries again.
    pub next_attempt_at: DateTime<Utc>,
}

/// An email address of an account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Email {
    /// The normalized address.
    pub address: String,
    /// When it was verified, if it was.
    pub verified_at: Option<DateTime<Utc>>,
    /// Whether it is the account's primary address.
    pub primary: bool,
}

/// An active browser session of an account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Session ID, the `sid` of the tokens issued from it.
    pub id: SessionId,
    /// How the session authenticated.
    pub methods: Vec<AuthenticationMethod>,
    /// When it signed in.
    pub authenticated_at: DateTime<Utc>,
    /// When it last proved possession of a passkey.
    pub passkey_confirmed_at: Option<DateTime<Utc>>,
    /// Last recorded activity.
    pub last_seen_at: DateTime<Utc>,
    /// When it expires.
    pub expires_at: DateTime<Utc>,
    /// The client whose device it is, when a device signed in to that client
    /// with a code; absent for a browser's session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<ClientId>,
    /// The admin dashboard's sessions resting on it, newest first: an
    /// administrator's, while they last.
    pub dashboard: Vec<DashboardSession>,
}

/// A session of the admin dashboard, which rests on an auth session: it lasts
/// while that one does, until it goes unused too long or reaches `expires_at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardSession {
    /// The auth session it rests on.
    pub session_id: SessionId,
    /// When the administrator signed in to the dashboard.
    pub signed_in_at: DateTime<Utc>,
    /// Its last request.
    pub last_used_at: DateTime<Utc>,
    /// When it ends, however busy.
    pub expires_at: DateTime<Utc>,
}

impl DashboardSession {
    /// Those of `sessions` resting on the auth session `session_id`, in order.
    #[must_use]
    pub fn on(sessions: &[Self], session_id: SessionId) -> Vec<Self> {
        sessions
            .iter()
            .filter(|session| session.session_id == session_id)
            .cloned()
            .collect()
    }
}

/// A browser that signed in to an account within the last 400 days, signed in
/// now or not, as its device cookie identifies it: a sign-in from any other one
/// alerts the account's owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownDevice {
    /// The browser and platform it last signed in with, if recorded.
    pub device: Option<UserAgentFamily>,
    /// When it first signed in.
    pub first_seen_at: DateTime<Utc>,
    /// When it last signed in.
    pub last_seen_at: DateTime<Utc>,
}

/// The scopes an account consented to for one application.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consent {
    /// The application's client ID.
    pub client_id: String,
    /// The audience of the tokens it receives.
    pub audience: String,
    /// The consented scopes.
    pub scopes: Vec<String>,
}

/// A signup invitation, without its secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invitation {
    /// Invitation ID.
    pub id: TokenId,
    /// The normalized address it was mailed to, which the account created with
    /// it starts with. `None` for a link administrators passed on themselves.
    pub email: Option<String>,
    /// When it was created.
    pub created_at: DateTime<Utc>,
    /// When it stops being accepted.
    pub expires_at: DateTime<Utc>,
    /// When an account redeemed it.
    pub redeemed_at: Option<DateTime<Utc>>,
    /// When it was revoked.
    pub revoked_at: Option<DateTime<Utc>>,
}

impl Invitation {
    /// Whether it can still be used, as of `now`.
    #[must_use]
    pub fn state(&self, now: DateTime<Utc>) -> InvitationState {
        if self.redeemed_at.is_some() {
            InvitationState::Redeemed
        } else if self.revoked_at.is_some() {
            InvitationState::Revoked
        } else if self.expires_at <= now {
            InvitationState::Expired
        } else {
            InvitationState::Open
        }
    }
}

/// Whether an invitation can still be used: the first that applies. The
/// database finds and redeems only an open one, by the same rule in SQL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvitationState {
    /// An account was created with it.
    Redeemed,
    /// An administrator or operator revoked it.
    Revoked,
    /// Its link expired unused.
    Expired,
    /// Its link works.
    Open,
}

/// A new invitation, and how its link reaches the person it invites.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedInvitation {
    /// Invitation ID, to resend or revoke it.
    pub id: TokenId,
    /// How its link reaches them.
    pub sent: InvitationSent,
    /// When the link stops working.
    pub expires_at: DateTime<Utc>,
}

/// How a new invitation's single-use link reaches the person it invites.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvitationSent {
    /// The link, shown only this once, for administrators to pass on.
    Link(Url),
    /// Mailed to this normalized address. Nobody else sees the link.
    Mailed(String),
}

/// An OIDC client: everything but its secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    /// Client ID.
    pub id: ClientId,
    /// Name shown on consent screens.
    pub display_name: String,
    /// Allowed authorization redirect URIs.
    pub redirect_uris: Vec<String>,
    /// Allowed post-logout redirect URIs.
    pub post_logout_uris: Vec<String>,
    /// Scopes it may request.
    pub allowed_scopes: Vec<String>,
    /// Audiences it may request tokens for.
    pub allowed_audiences: Vec<String>,
    /// The grants it may use at the token endpoint, under their RFC 7591 name.
    pub grant_types: Vec<GrantType>,
    /// Whether it is usable.
    pub enabled: bool,
    /// Whether it skips the consent screen.
    pub trusted: bool,
    /// Whether it must push its authorization requests (RFC 9126). Absent from
    /// auth releases that predate them.
    #[serde(default)]
    pub require_pushed_authorization_requests: bool,
    /// How it authenticates at the token endpoint.
    pub token_endpoint_auth_method: ClientAuthMethod,
    /// Where logout tokens are posted when a session it holds tokens from ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backchannel_logout_uri: Option<String>,
    /// The account that manages it; absent for clients operators manage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<UserId>,
    /// When a token was last issued to it, to within an hour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    /// When an operator suspended it: it stays disabled, and its owner cannot
    /// enable it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspended_at: Option<DateTime<Utc>>,
    /// When it was deleted. It stays listed, disabled, so its ID is never reused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Client {
    /// Whether it signs users in, and why not.
    #[must_use]
    pub const fn status(&self) -> ClientStatus {
        ClientStatus::of(self.enabled, self.suspended_at, self.deleted_at)
    }
}

/// One OIDC client as its page and `client show` show it: the client as listed,
/// with its kind, when it was registered and changed, and its secrets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientDetail {
    /// The client, as the list shows it.
    #[serde(flatten)]
    pub client: Client,
    /// Public, or confidential with secrets.
    pub kind: ClientKind,
    /// When it was registered.
    pub created_at: DateTime<Utc>,
    /// When its registration last changed.
    pub updated_at: DateTime<Utc>,
    /// A confidential client's secrets, oldest first, expired ones included.
    pub secrets: Vec<SecretLifetime>,
}

/// One of a confidential client's secrets: when it was made and when it stops
/// working, never its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretLifetime {
    /// The secret.
    pub id: ClientSecretId,
    /// When it was made.
    pub created_at: DateTime<Utc>,
    /// When a rotation made it stop working, if one did.
    pub expires_at: Option<DateTime<Utc>>,
}

/// Where an account recovery stands. The database crate computes it as it reads
/// one; the migration's `undoable_recovery` is the one SQL copy, which the
/// database's backstops use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecoveryState {
    /// Its link works: not used, cancelled or undone, and not expired.
    Open,
    /// Its link was used and its probation has not ended: its passkey signs in
    /// but never confirms a session, and a passkey from before it can undo it.
    Probation {
        /// When the probation ends.
        settles_at: DateTime<Utc>,
    },
    /// Its probation ended: its passkey is an ordinary passkey.
    Settled,
    /// It expired unused, or was cancelled or undone.
    Ended,
}

impl RecoveryState {
    /// Whether the recovery can still be undone: its link works, or it is in
    /// probation.
    #[must_use]
    pub const fn undoable(self) -> bool {
        matches!(self, Self::Open | Self::Probation { .. })
    }
}

/// A recent recovery of an account, as support reviews it before starting
/// another: where it stands, and the passkey its link enrolled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recovery {
    /// The recovery.
    pub id: RecoveryId,
    /// When it started.
    pub started_at: DateTime<Utc>,
    /// When its link stops, or stopped, working unused.
    pub expires_at: DateTime<Utc>,
    /// When its link enrolled a passkey, if it did.
    pub completed_at: Option<DateTime<Utc>>,
    /// When it was cancelled before its link was used.
    pub cancelled_at: Option<DateTime<Utc>>,
    /// When it was undone, by its owner or by support.
    pub undone_at: Option<DateTime<Utc>>,
    /// Where it stands now.
    pub state: RecoveryState,
    /// The passkey its link enrolled, if it did.
    pub passkey: Option<RecoveryPasskey>,
}

impl Recovery {
    /// Where it stands, as people read it: its state, with why one ended.
    #[must_use]
    pub const fn standing(&self) -> RecoveryStanding {
        match self.state {
            RecoveryState::Open => RecoveryStanding::Unused,
            RecoveryState::Probation { settles_at } => RecoveryStanding::Probation { settles_at },
            RecoveryState::Settled => RecoveryStanding::Settled,
            RecoveryState::Ended => match (self.undone_at, self.cancelled_at) {
                (Some(at), _) => RecoveryStanding::Undone(at),
                (None, Some(at)) => RecoveryStanding::Cancelled(at),
                (None, None) => RecoveryStanding::Expired,
            },
        }
    }
}

/// Where a recovery stands, as the dashboard and the CLI say it: its
/// [`RecoveryState`], with an ended one split by why it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryStanding {
    /// Its link works, and was not used yet.
    Unused,
    /// Its link was used: its passkey signs in, confirms no change, and the
    /// recovery can be undone until `settles_at`.
    Probation {
        /// When the probation ends.
        settles_at: DateTime<Utc>,
    },
    /// Its probation ended.
    Settled,
    /// It was undone, after its link was used.
    Undone(DateTime<Utc>),
    /// It was cancelled before its link was used.
    Cancelled(DateTime<Utc>),
    /// Its link expired unused.
    Expired,
}

/// The passkey a recovery's link enrolled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPasskey {
    /// The passkey.
    pub id: PasskeyId,
    /// Its label, while its row exists.
    pub friendly_name: Option<String>,
    /// Whether it is still active.
    pub active: bool,
}

/// The event list's query. Every filter is optional.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSearch {
    /// Only events concerning this account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<UserId>,
    /// Only events of this kind, e.g. `session_revoked`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Whether administrators' views of accounts are listed too. They are left
    /// out unless asked for, here or as the `kind`, so the list shows changes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub views: bool,
    /// The page wanted, counting from 1 at the newest; the first when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

/// One page of domain events, newest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPage {
    /// The matching events.
    pub events: Vec<Event>,
    /// Where the page stands among every match.
    pub paging: Paging,
}

/// A recorded domain event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Event ID.
    pub id: EventId,
    /// When it was recorded.
    pub occurred_at: DateTime<Utc>,
    /// Who caused it, e.g. `{"type": "admin", "user_id": …, "session_id": …}`.
    pub actor: serde_json::Value,
    /// The current handle of the administrator who caused it, when one did and
    /// their account still exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub administrator: Option<Handle>,
    /// The account it concerns, if any.
    pub subject: Option<UserId>,
    /// The trace of the request or job that recorded it, to look up in telemetry
    /// while the trace is kept. Absent for events recorded outside a trace, such as
    /// by the command line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<TraceId>,
    /// What happened: `{"kind": …, "data": …}`, as auth stores it. Personal values
    /// read `{"$personal": value}`, or `"$erased"` once deleted. An event of a kind
    /// or payload version the serving release does not know, written by a newer
    /// one, also carries its `version`.
    pub event: serde_json::Value,
    /// Why the administrator or operator who caused it did, as they wrote it,
    /// while kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The request's IP address, while kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<IpAddr>,
    /// The request's browser and platform, while kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<UserAgentFamily>,
    /// Whether retention or a purge deleted its personal data: the IP address,
    /// device and reason.
    #[serde(default)]
    pub erased: bool,
}

impl Event {
    /// Who caused it.
    ///
    /// # Errors
    ///
    /// Returns why its stored `actor` is not one this release knows.
    #[cfg(feature = "events")]
    pub fn actor(&self) -> Result<crate::events::Actor, serde_json::Error> {
        crate::events::Actor::deserialize(&self.actor)
    }

    /// Its kind, e.g. `session_revoked`, if its stored `event` names one.
    #[must_use]
    pub fn kind(&self) -> Option<&str> {
        self.event.get("kind").and_then(serde_json::Value::as_str)
    }
}

/// An origin approved for a client's pages to sign users in with passkeys, which
/// `/.well-known/webauthn` lists as a `WebAuthn` related origin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedOrigin {
    /// The origin, as browsers send it.
    pub origin: RelatedOrigin,
    /// The client whose pages are on it.
    pub client_id: ClientId,
    /// When it was approved.
    pub approved_at: DateTime<Utc>,
}

/// An administrator account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Administrator {
    /// The administrator's account.
    pub user_id: UserId,
    /// The account's handle.
    pub handle: Handle,
    /// The account's profile picture, served at `/pictures/{id}` on auth, while the
    /// account is active: auth serves it only then.
    pub picture: Option<PictureId>,
    /// When the rights were granted.
    pub granted_at: DateTime<Utc>,
}

/// An announcement to accounts, such as a security incident or a change to the
/// privacy policy: a draft until it is sent, and unchanged once it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announcement {
    /// Announcement ID.
    pub id: AnnouncementId,
    /// The email's subject, and its title in the recipients' settings.
    pub subject: String,
    /// Its text, as typed: paragraphs separated by blank lines.
    pub body: String,
    /// Whether it goes to every account, rather than to chosen ones.
    pub everyone: bool,
    /// Whether it is emailed to every recipient with a primary verified address,
    /// whatever their preference, as a security incident or a change to the
    /// privacy policy or terms must be.
    pub essential: bool,
    /// Counts its draft's changes: sending names the one its sender checked.
    pub revision: u64,
    /// How many accounts it goes to: the chosen ones, or every account, as of
    /// now for a draft and as of its sending once sent. Purged accounts no longer
    /// count.
    pub recipients: u64,
    /// Whether auth still knows whom it went to: cleanup deletes its recipients
    /// a year after it is sent, and `recipients`, with how many were mailed or
    /// read it, then count none.
    pub recipients_kept: bool,
    /// When it was drafted.
    pub created_at: DateTime<Utc>,
    /// When its draft last changed.
    pub updated_at: DateTime<Utc>,
    /// When it was sent; `None` while it is a draft.
    pub sent_at: Option<DateTime<Utc>>,
}

impl Announcement {
    /// Whom its email reaches, after how many: e.g. "12 with a primary verified
    /// address".
    #[must_use]
    pub const fn emailed(&self) -> &'static str {
        if self.essential {
            "with a primary verified address"
        } else {
            "with a primary verified address who receive announcements by email"
        }
    }

    /// Whether it is essential, as its pages and the CLI say it.
    #[must_use]
    pub const fn essentiality(&self) -> &'static str {
        if self.essential {
            "Yes: emailed whatever the recipient's preference"
        } else {
            "No: recipients who turned announcement emails off only find it in their inbox"
        }
    }

    /// The paragraphs of an announcement's text `body`, split at blank lines, each
    /// a list of its lines, so every page and email shows the line breaks it was
    /// written with.
    #[must_use]
    pub fn paragraphs(body: &str) -> Vec<Vec<&str>> {
        let mut paragraphs = Vec::new();
        let mut paragraph = Vec::new();
        for line in body.lines().map(str::trim_end) {
            if line.trim().is_empty() {
                if !paragraph.is_empty() {
                    paragraphs.push(std::mem::take(&mut paragraph));
                }
            } else {
                paragraph.push(line);
            }
        }
        if !paragraph.is_empty() {
            paragraphs.push(paragraph);
        }
        paragraphs
    }
}

/// One announcement, with who its email reaches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnouncementDetail {
    /// The announcement.
    pub announcement: Announcement,
    /// The accounts it was chosen for, by handle; empty for one to everyone.
    pub accounts: Vec<User>,
    /// How many of its recipients have a primary verified address, which its
    /// email goes to, as of now: the others only see it in their inbox.
    pub mailed: u64,
    /// How many of its recipients have read it in their inbox, once it is sent.
    pub read: u64,
    /// Its emails in the delivery queue, by status, once it is sent. The queue
    /// deletes them 7 days after they succeed, 30 after they fail.
    pub emails: AnnouncementEmails,
}

/// An announcement's emails in the delivery queue, by status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnouncementEmails {
    /// Waiting for an attempt, or being attempted.
    pub pending: u64,
    /// Accepted by the relay.
    pub succeeded: u64,
    /// Refused for good, or past their retries.
    pub failed: u64,
    /// Cancelled by an operator.
    pub cancelled: u64,
}

/// The send page's query: the revision of the draft its sender checked, so a
/// change made meanwhile is not sent unseen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendAnnouncement {
    /// The draft's revision, as its page showed it.
    pub revision: u64,
}

/// The result of verifying the audit chain, as `auth-service audit verify` prints
/// it and the audit log records it, as `DomainEvent::AuditChainVerified`, for
/// the dashboard's audit page to show. Verification stops at the first problem.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditVerification {
    /// The first position that does not match, or `None` if the chain is intact.
    pub broken: Option<ChainBreak>,
    /// Sealed events whose hash was recomputed.
    pub events_checked: u64,
    /// Checkpoints whose signature was checked.
    pub checkpoints_checked: u64,
    /// Events not sealed yet. They are sealed within seconds, once every older
    /// transaction has finished.
    pub unsealed: u64,
    /// The checkpoint the chain was last pruned through, which it extends.
    pub anchor: Option<ChainPoint>,
    /// The newest sealed event.
    pub newest: Option<ChainPoint>,
    /// The newest checkpoint.
    pub latest_checkpoint: Option<ChainPoint>,
    /// The keys checkpoint signatures were checked against.
    pub keys: VerificationKeys,
}

/// A position on the audit chain and the chain hash through it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainPoint {
    /// The position, from 1.
    pub position: u64,
    /// The chain hash, in standard base64 as checkpoints write it.
    pub hash: String,
}

impl std::fmt::Display for ChainPoint {
    /// Writes the position and hash, e.g. `3037 b3Fx…=`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} {}", self.position, self.hash)
    }
}

/// Where the audit chain stops matching.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainBreak {
    /// The first position that does not match. The change may also be a removal
    /// or insertion just before it.
    pub position: u64,
    /// What does not match there.
    pub problem: ChainProblem,
}

/// What does not match at a [`ChainBreak`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainProblem {
    /// No sealed event has the position.
    MissingEvent,
    /// A sealed event remains at or before the anchor, which pruning would have
    /// deleted: the anchor was moved to hide it.
    EventBeforeAnchor,
    /// The event's content or position does not match its stored chain hash.
    EventChanged,
    /// The event was sealed out of the order events were recorded in, which
    /// histories list them in: its transaction and sequence, which the chain does
    /// not cover, do not come after the previous event's.
    OutOfOrder,
    /// The chain recomputed through the position does not match the checkpoint
    /// signed there: an event before it was changed and the stored hashes
    /// recomputed.
    CheckpointMismatch,
    /// The checkpoint's signature does not verify against the keys.
    CheckpointSignature,
    /// The checkpoint's note does not match its stored position and head.
    CheckpointMalformed,
    /// A checkpoint was signed at the position, but the chain ends before it:
    /// newer events were deleted.
    CheckpointBeyondChain,
}

impl std::fmt::Display for ChainProblem {
    /// Explains the problem in a sentence fragment.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::MissingEvent => "no sealed event has this position",
            Self::EventBeforeAnchor => {
                "a sealed event remains here, at or before the anchor it was pruned through"
            }
            Self::EventChanged => "the event does not match its chain hash",
            Self::OutOfOrder => {
                "the event's recorded order does not come after the previous event's"
            }
            Self::CheckpointMismatch => {
                "the recomputed chain does not match the checkpoint signed here"
            }
            Self::CheckpointSignature => "the checkpoint's signature does not verify",
            Self::CheckpointMalformed => {
                "the checkpoint's note does not match its stored position and head"
            }
            Self::CheckpointBeyondChain => {
                "a checkpoint was signed here, but the chain ends before it"
            }
        })
    }
}

/// The keys checkpoint signatures were checked against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationKeys {
    /// The keys committed to the repository (`docs/dev/security/audit-key.pub`).
    Committed,
    /// Keys the operator gave on the command line.
    Given,
    /// The server's own key, since none is committed for its environment, as on a
    /// preview. A compromised server could have signed with it.
    Uncommitted,
}

impl std::fmt::Display for VerificationKeys {
    /// Describes the keys.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Committed => "committed",
            Self::Given => "given with --key",
            Self::Uncommitted => "this server's own, which is not committed",
        })
    }
}

/// The dashboard's audit page: the chain as stored, the newest verification the
/// audit log records, and one page of checkpoints, oldest first, after the anchor.
/// Reading it recomputes nothing, so it costs the same however long the history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditPage {
    /// The chain as stored, not verified.
    pub chain: StoredChain,
    /// The newest verification recorded, if any.
    pub verified: Option<RecordedVerification>,
    /// The checkpoints, judged by `verified`.
    pub checkpoints: Vec<AuditCheckpoint>,
    /// The position to continue after, when there may be more.
    pub next: Option<u64>,
}

/// The audit chain as stored, read without recomputing or checking anything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredChain {
    /// The checkpoint the chain was last pruned through, which it extends.
    pub anchor: Option<ChainPoint>,
    /// The newest sealed event, or the anchor once every sealed event was pruned.
    pub newest: Option<ChainPoint>,
    /// Events not sealed yet.
    pub unsealed: u64,
}

/// A verification of the audit chain, and when the audit log recorded it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedVerification {
    /// When it was recorded, just after it finished.
    pub at: DateTime<Utc>,
    /// What it found.
    pub verification: AuditVerification,
}

/// A signed checkpoint, and whether the chain may be pruned through it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditCheckpoint {
    /// Its position and the head it signs.
    pub point: ChainPoint,
    /// When it was signed.
    pub signed_at: DateTime<Utc>,
    /// Whether `auth-service audit prune --through` its position would prune.
    pub pruning: Pruning,
}

/// Whether the chain may be pruned through a checkpoint, by the rules `audit
/// prune` applies, as far as a verification shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pruning {
    /// It may: the anchor precedes it, and the verification found no break at or
    /// before it.
    Prunable,
    /// The chain was already pruned through it or a later checkpoint.
    Pruned {
        /// The anchor's position.
        anchor: u64,
    },
    /// The chain breaks at or before it.
    Broken(ChainBreak),
}

impl Pruning {
    /// Whether the chain, pruned through `anchor` and found `broken` by a
    /// verification, may be pruned through the checkpoint at `position`. `audit
    /// prune` judges by a verification it just made, and the dashboard's audit page
    /// by the newest one recorded.
    #[must_use]
    pub fn of(anchor: Option<&ChainPoint>, broken: Option<&ChainBreak>, position: u64) -> Self {
        match (anchor, broken) {
            (Some(anchor), _) if anchor.position >= position => Self::Pruned {
                anchor: anchor.position,
            },
            (_, Some(broken)) if broken.position <= position => Self::Broken(broken.clone()),
            _ => Self::Prunable,
        }
    }
}

crate::str_enum! {
    /// Where a delivery is in its life.
    pub enum DeliveryStatus {
        /// Waiting for its next attempt, or being attempted.
        Pending = "pending",
        /// The receiver accepted it.
        Succeeded = "succeeded",
        /// A permanent error, or retryable errors past the channel's maximum age.
        Failed = "failed",
        /// An operator cancelled it.
        Cancelled = "cancelled",
    }
}

/// A delivery as operators list it, without its recipient or payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliverySummary {
    /// Delivery identifier.
    pub id: DeliveryId,
    /// Its channel's name.
    pub channel: String,
    /// Where it goes.
    pub destination: String,
    /// Where it is in its life.
    pub status: DeliveryStatus,
    /// Attempts so far.
    pub attempts: u64,
    /// When the next attempt is due, while pending.
    pub next_attempt_at: DateTime<Utc>,
    /// The latest attempt's error.
    pub last_error: Option<String>,
    /// When it was created.
    pub created_at: DateTime<Utc>,
    /// When it succeeded, failed or was cancelled.
    pub finished_at: Option<DateTime<Utc>>,
}

impl DeliverySummary {
    /// Whether an operator may attempt it again: once it failed or was cancelled.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(
            self.status,
            DeliveryStatus::Failed | DeliveryStatus::Cancelled
        )
    }

    /// Whether an operator may cancel it: while pending or failed.
    #[must_use]
    pub const fn cancellable(&self) -> bool {
        matches!(
            self.status,
            DeliveryStatus::Pending | DeliveryStatus::Failed
        )
    }
}

crate::str_enum! {
    /// Where a signing key is in its lifecycle.
    pub enum SigningKeyState {
        /// Published ahead of use, so verifiers have it before it signs.
        Next = "next",
        /// Signs new tokens; one per key type.
        Active = "active",
        /// No longer signs, but stays published while its tokens may be valid.
        Retiring = "retiring",
        /// Unpublished, its private key erased; verifies only `id_token_hint`s.
        Retired = "retired",
    }
}

crate::str_enum! {
    /// The family of a signing key, which fixes the algorithms it signs with.
    pub enum SigningKeyType {
        /// RSA, signing RS256.
        Rsa = "rsa",
    }
}

/// A signing key, without its public or private key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SigningKey {
    /// The key's `kid`.
    pub id: KeyId,
    /// Its family.
    pub key_type: SigningKeyType,
    /// Where it is in its lifecycle.
    pub state: SigningKeyState,
    /// When it was generated.
    pub created_at: DateTime<Utc>,
    /// When it started signing.
    pub activated_at: Option<DateTime<Utc>>,
    /// When it stopped signing.
    pub deactivated_at: Option<DateTime<Utc>>,
    /// When it left the key set.
    pub retired_at: Option<DateTime<Utc>>,
}

/// A handle an administrator reserved: nobody takes it, or a look-alike, until an
/// administrator gives it to an account or removes it from the list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservedHandle {
    /// The reserved handle.
    pub handle: Handle,
    /// Who or what it is kept for.
    pub reason: String,
    /// When it was reserved.
    pub reserved_at: DateTime<Utc>,
}

/// A handle no account holds, but that is kept from others until an operator
/// releases it: one an account held before, which only it may claim again, or a
/// purged account's last one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleasedHandle {
    /// The handle.
    pub handle: Handle,
    /// The account that held it, or `None` once purged.
    pub user_id: Option<UserId>,
    /// When the account stopped using it, or `None` for a purged account's last one.
    pub released_at: Option<DateTime<Utc>>,
}

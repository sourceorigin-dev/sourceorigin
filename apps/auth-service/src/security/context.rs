//! Where a request came from, which
//! [`HttpSecurity::protect`](super::HttpSecurity::protect) attaches, and the event
//! contexts that attribute a request's events.

use super::{ApiError, HttpSecurityError};
use axum::{
    extract::FromRequestParts,
    http::{Extensions, HeaderMap, header, request::Parts},
};
use sourceorigin_common::{
    AuthenticationMethod, ClientId, UserId,
    events::{Actor, DomainEvent, EventContext, RequestOrigin, SignInFailure, UserAgentFamily},
};
use sourceorigin_repository::{Database, DatabaseError, DatabaseTransaction, StoredSession};
use sourceorigin_telemetry::analytics::Consent;
use std::net::IpAddr;

/// Where a request came from, attached by
/// [`HttpSecurity::protect`](super::HttpSecurity::protect) to every rate-limited
/// route and extracted by handlers that record events and by the session
/// extractors. The raw `User-Agent` header is classified, never kept.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RequestContext {
    /// The client's IP address.
    pub client_ip: IpAddr,
    /// The coarse browser and platform of its `User-Agent` header.
    pub user_agent_family: UserAgentFamily,
    /// Whether the browser asked for Global Privacy Control (`Sec-GPC: 1`), which
    /// keeps it out of usage statistics.
    pub gpc: bool,
}

impl RequestContext {
    /// The client IP and the coarse family of its `User-Agent` header.
    pub(super) fn new(client_ip: IpAddr, headers: &HeaderMap) -> Self {
        Self {
            client_ip,
            user_agent_family: UserAgentFamily::classify(
                headers
                    .get(header::USER_AGENT)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default(),
            ),
            gpc: headers
                .get(Consent::GPC_HEADER)
                .is_some_and(|value| value.as_bytes() == b"1"),
        }
    }

    /// Consent to record a product analytics row about `user`'s activity in
    /// this request, unless the account left usage statistics or the browser
    /// asked for Global Privacy Control.
    pub(crate) async fn consent(
        &self,
        tx: &mut DatabaseTransaction<'_>,
        user: UserId,
    ) -> Result<Option<Consent>, DatabaseError> {
        let included = !self.gpc && tx.users().analytics_included(user).await?;
        Ok(Consent::given(included, self.gpc))
    }

    /// Attributes events of this request to `actor`.
    pub(crate) fn actor(self, actor: Actor) -> EventContext {
        EventContext {
            actor,
            origin: RequestOrigin {
                client_ip: Some(self.client_ip),
                user_agent_family: Some(self.user_agent_family),
            },
            reason: None,
        }
    }

    /// Attributes events of this request to the user signed in with `session`.
    pub(crate) fn user(self, session: &StoredSession) -> EventContext {
        self.actor(Actor::User {
            user_id: session.user_id,
            session_id: Some(session.id),
        })
    }

    /// Attributes events of this request to account `user_id` acting without a
    /// session, such as through an emailed link, or with a passkey before it
    /// signs in.
    pub(crate) fn user_without_session(self, user_id: UserId) -> EventContext {
        self.actor(Actor::User {
            user_id,
            session_id: None,
        })
    }

    /// Attributes events of this request to an authenticated client.
    pub(crate) fn client(self, client_id: &ClientId) -> EventContext {
        self.actor(Actor::Client {
            client_id: client_id.clone(),
        })
    }

    /// Records a rejected authentication in a transaction of its own, since the
    /// attempt changed nothing. `subject` is the account it targeted, when known.
    pub(crate) async fn record_sign_in_failure(
        self,
        database: &Database,
        actor: Actor,
        subject: Option<UserId>,
        method: AuthenticationMethod,
        reason: SignInFailure,
    ) -> Result<(), DatabaseError> {
        let mut tx = database.begin().await?;
        tx.record(
            &self.actor(actor),
            subject,
            DomainEvent::SignInFailed { method, reason },
        )
        .await?;
        tx.commit().await
    }

    /// The context [`HttpSecurity::protect`](super::HttpSecurity::protect) attached to a
    /// request's `extensions`. Only routes with a rate budget have one.
    pub(crate) fn of(extensions: &Extensions) -> Result<Self, HttpSecurityError> {
        extensions
            .get::<Self>()
            .copied()
            .ok_or(HttpSecurityError::MissingContext)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for RequestContext {
    type Rejection = ApiError;
    /// Reads the context [`HttpSecurity::protect`](super::HttpSecurity::protect) attached.
    fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Self::of(&parts.extensions).map_err(ApiError::from))
    }
}

//! The device authorization grant (RFC 8628). A device without a browser, such as
//! a command-line tool, asks for a device code and a user code, shows its user the
//! user code, and polls the token endpoint with the device code. The user types
//! the user code on auth's device page, signed in on another device, and decides.
//! An approval signs the device in, in a session of its own.

use super::{
    GrantType, OAuthCode, OAuthError, OidcApplication, OpaqueToken, Reject, Scopes, TokenPrefix,
    policy::ClientPolicy,
    token::{ACCOUNT_DISABLED, CONSENT_WITHDRAWN, SESSION_ENDED, TokenGrant},
};
use crate::{
    routes,
    security::{Budget, RequestContext, SESSION_LIFETIME_DAYS, Session, SessionError},
};
use axum::{
    Form, Json,
    extract::{State, rejection::FormRejection},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use sourceorigin_common::{
    ClientId, Handle, SessionId, TokenId, UserId,
    admin::GrantedResource,
    events::DomainEvent,
    protocol::oauth::{DeviceAuthorization, TokenResponse},
};
use sourceorigin_crypto::{CryptoError, Secret};
use sourceorigin_repository::{
    DatabaseError, DatabaseTransaction, DeviceAuthorizationState, InsertDeviceAuthorization,
    InsertRefreshFamily, PendingDeviceAuthorization, PolledDeviceAuthorization,
    SessionAuthentication, StoredSession,
};
use sourceorigin_ui::DeviceCodeQuery;
use std::fmt;

/// How long a device authorization's two codes are accepted.
const DEVICE_CODE_SECONDS: u32 = 600;
/// Seconds a device leaves between two polls of the token endpoint.
const POLL_SECONDS: u16 = 5;
/// Seconds a poll that comes too soon adds to that interval (RFC 8628 §3.5).
const SLOW_DOWN_SECONDS: u16 = 5;
/// How many user codes are drawn before giving up on one no other pending
/// authorization holds. A collision is one in billions, so one draw nearly
/// always does.
const USER_CODE_DRAWS: usize = 3;
/// Letters a user code is drawn from: RFC 8628 §6.1's, which spell no word and
/// are told apart whatever their case.
const USER_CODE_ALPHABET: [u8; 20] = *b"BCDFGHJKLMNPQRSTVWXZ";
/// Letters in a user code, so 20^8 codes: about 34.5 bits.
const USER_CODE_LETTERS: usize = 8;
/// Separates the digests of user codes from those of opaque tokens' secrets.
const USER_CODE_CONTEXT: &[u8] = b"sourceorigin-device-user-code-v1";
/// Lookups by user code a browser may make, per IP address and per session: a
/// code is short enough to guess, so guesses are counted apart from the page's
/// other requests.
const LOOKUPS: Budget = Budget {
    name: "device",
    per_minute: 10,
};

/// The code a device shows its user, who types it on auth's device page: eight
/// letters of [`USER_CODE_ALPHABET`], shown as `BDFG-HJKL`. Only its digest is
/// stored.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct UserCode([u8; USER_CODE_LETTERS]);

impl UserCode {
    /// Draws a code from the operating system's randomness, every letter as
    /// likely as any other.
    fn generate() -> Result<Self, CryptoError> {
        /// How many letters there are, as a byte.
        const LETTERS: u8 = 20;
        /// Bytes from here up are dropped: counting them would favour the first
        /// letters, since 256 is no multiple of 20.
        const LIMIT: u8 = u8::MAX - u8::MAX % LETTERS;
        loop {
            let random = Secret::generate()?;
            let mut letters = random
                .as_bytes()
                .iter()
                .filter(|byte| **byte < LIMIT)
                .map(|byte| USER_CODE_ALPHABET[usize::from(byte % LETTERS)]);
            let mut code = [0; USER_CODE_LETTERS];
            if code
                .iter_mut()
                .all(|slot| letters.next().map(|letter| *slot = letter).is_some())
            {
                return Ok(Self(code));
            }
        }
    }

    /// Reads a code as its user typed it, whatever its case, dashes and spaces.
    /// `None` unless it is eight letters of the alphabet.
    pub(crate) fn parse(typed: &str) -> Option<Self> {
        let mut letters = typed
            .bytes()
            .filter(|byte| !(byte.is_ascii_whitespace() || *byte == b'-'))
            .map(|byte| byte.to_ascii_uppercase());
        let mut code = [0; USER_CODE_LETTERS];
        for slot in &mut code {
            *slot = letters
                .next()
                .filter(|letter| USER_CODE_ALPHABET.contains(letter))?;
        }
        letters.next().is_none().then_some(Self(code))
    }

    /// The code as a secret of the same length as an opaque token's, so it is
    /// digested under the token digest keys as one is.
    fn secret(self) -> Secret {
        Secret::derive(USER_CODE_CONTEXT, &self.0)
    }
}

impl fmt::Display for UserCode {
    /// Writes the code as devices show it, such as `BDFG-HJKL`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (first, second) = self.0.split_at(USER_CODE_LETTERS / 2);
        for letter in first.iter().chain(b"-").chain(second) {
            write!(formatter, "{}", char::from(*letter))?;
        }
        Ok(())
    }
}

impl fmt::Debug for UserCode {
    /// Writes nothing of the code, which approves a sign-in.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UserCode(REDACTED)")
    }
}

/// The parameters of a device authorization request (RFC 8628 §3.1).
#[derive(Deserialize)]
pub(super) struct DeviceAuthorizationForm {
    /// Registered relying-party identifier.
    client_id: Option<String>,
    /// Space-separated scopes the device asks for.
    #[serde(default)]
    scope: String,
    /// The resource the access token is for (RFC 8707); the client itself when
    /// absent.
    resource: Option<String>,
}

/// The parameters of a device code grant (RFC 8628 §3.4).
#[derive(Deserialize)]
pub(super) struct DeviceRedemption {
    /// The device code the device authorization answered with.
    device_code: String,
    /// The resource the authorization named, if the device repeats it.
    resource: Option<String>,
}

/// A device authorization as the device page shows it for a decision.
pub(crate) struct DeviceRequest {
    /// The name of the client whose device asks.
    pub(crate) client: String,
    /// The handle of the account that registered the client, which nobody
    /// verified; `None` for a client operators registered.
    pub(crate) registered_by: Option<Handle>,
    /// The handle of the account the browser is signed in to, which would approve.
    pub(crate) account: Handle,
    /// What the device asks to receive.
    pub(crate) scopes: Vec<String>,
    /// The API the device asks tokens for, if not the client itself, with the
    /// scopes of it asked for.
    pub(crate) resource: Option<GrantedResource>,
    /// When the device asked.
    pub(crate) started_at: DateTime<Utc>,
}

/// What the device page's user decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceDecision {
    /// Sign the device in.
    Approve,
    /// Refuse it.
    Deny,
}

/// What looking a user code up found.
pub(crate) enum DeviceLookup<T> {
    /// The device authorization it names, which waits for a decision.
    Found(T),
    /// No device authorization waits under this code: it was mistyped, it
    /// expired, or it was already decided on.
    Unknown,
    /// The browser spent its budget of lookups, and may try again in a minute.
    Throttled,
}

impl<T> DeviceLookup<T> {
    /// The same answer, with what was found as `shown` shows it.
    pub(crate) fn map<U>(self, shown: impl FnOnce(T) -> U) -> DeviceLookup<U> {
        match self {
            Self::Found(found) => DeviceLookup::Found(shown(found)),
            Self::Unknown => DeviceLookup::Unknown,
            Self::Throttled => DeviceLookup::Throttled,
        }
    }
}

impl OidcApplication {
    /// Starts a device authorization for an authenticated client registered for
    /// the device grant: stores it, and answers its two codes, the page where the
    /// user types theirs and how often the device may poll.
    pub(super) async fn authorize_device(
        State(app): State<Self>,
        headers: HeaderMap,
        form: Result<Form<DeviceAuthorizationForm>, FormRejection>,
    ) -> Result<Json<DeviceAuthorization>, OAuthError> {
        let Form(form) =
            form.map_err(|error| OAuthError::malformed("device authorization request", error))?;
        let client = app
            .authenticate_client(&headers, form.client_id.as_deref())
            .await?;
        Self::registered_for(&client, GrantType::DeviceCode)?;
        let (audience, scopes) = Scopes::requested(&form.scope, form.resource.as_deref(), &client)?;
        let scopes = scopes.into_stored();
        let device_code = OpaqueToken::new(TokenPrefix::DeviceCode)?;
        let (secret_digest, digest_key) = app.token_digest(&device_code);
        let mut authorization = InsertDeviceAuthorization {
            id: device_code.id,
            secret_digest,
            user_code_digest: Vec::new(),
            digest_key,
            client_id: client.id,
            scopes,
            audience,
            poll_interval: POLL_SECONDS,
            expires_at: Utc::now() + Duration::seconds(DEVICE_CODE_SECONDS.into()),
        };
        let user_code = app.store_device_authorization(&mut authorization).await?;
        let page = DeviceCodeQuery::default();
        let complete = DeviceCodeQuery {
            user_code: Some(user_code.to_string()),
        };
        Ok(Json(DeviceAuthorization {
            device_code: device_code.encoded(),
            user_code: user_code.to_string(),
            verification_uri: app.page(routes::page::DEVICE, &page)?,
            verification_uri_complete: app.page(routes::page::DEVICE, &complete)?,
            expires_in: DEVICE_CODE_SECONDS,
            interval: POLL_SECONDS.into(),
        }))
    }

    /// Stores `authorization` under a user code no other one holds, and returns
    /// that code.
    async fn store_device_authorization(
        &self,
        authorization: &mut InsertDeviceAuthorization,
    ) -> Result<UserCode, OAuthError> {
        let mut draws = 0;
        loop {
            let user_code = UserCode::generate()?;
            let (digest, _) = self.digest_keys.issue(&user_code.secret());
            authorization.user_code_digest = digest.as_bytes().to_vec();
            draws += 1;
            let stored = self
                .database
                .oidc()
                .insert_device_authorization(authorization)
                .await;
            match stored {
                Ok(()) => return Ok(user_code),
                Err(DatabaseError::Conflict(_)) if draws < USER_CODE_DRAWS => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// The device authorization waiting under `code`, as the device page shows it
    /// to the browser's signed-in account for a decision. The lookup counts in the
    /// browser's budget, whatever it finds.
    pub(crate) async fn device_request(
        &self,
        context: RequestContext,
        headers: &HeaderMap,
        code: UserCode,
    ) -> Result<DeviceLookup<DeviceRequest>, OAuthError> {
        let session = Session::required(&self.database, headers).await?;
        if !self.spend_lookup(context, &session).await? {
            return Ok(DeviceLookup::Throttled);
        }
        let digests = self.digest_keys.accepted(&code.secret());
        let Some(pending) = self
            .database
            .oidc()
            .find_pending_device_authorization(&digests)
            .await?
        else {
            return Ok(DeviceLookup::Unknown);
        };
        ClientPolicy::read(&self.database, &pending.client_id)
            .await?
            .allows_device(&pending.scopes, &pending.audience)
            .map_err(|refusal| refusal.reject(OAuthCode::InvalidRequest))?;
        let registered_by = match pending.client_owner_id {
            Some(owner) => Some(self.handle(owner).await?.ok_or_else(Self::unregistered)?),
            None => None,
        };
        let account = self
            .handle(session.user_id)
            .await?
            .ok_or(SessionError::Missing)?;
        let resource = self.granted(&pending.audience, &pending.scopes).await?;
        Ok(DeviceLookup::Found(DeviceRequest {
            client: pending.client_display_name,
            registered_by,
            account,
            resource,
            scopes: pending.scopes,
            started_at: pending.created_at,
        }))
    }

    /// Records the decision of the browser's signed-in account on the device
    /// authorization waiting under `code`, and returns its client's name. An
    /// approval saves the consent and creates the device's own session, whose
    /// tokens the device's next poll collects. The lookup counts in the browser's
    /// budget, whatever it finds.
    ///
    /// It locks the account and the session first, as the consent page does, so
    /// no decision commits after its session ended. An approval then share-locks
    /// the client until it commits, so no change to the client lands between its
    /// policy's check and the device's session.
    pub(crate) async fn decide_device(
        &self,
        context: RequestContext,
        headers: &HeaderMap,
        code: UserCode,
        decision: DeviceDecision,
    ) -> Result<DeviceLookup<String>, OAuthError> {
        let session = Session::required(&self.database, headers).await?;
        if !self.spend_lookup(context, &session).await? {
            return Ok(DeviceLookup::Throttled);
        }
        let digests = self.digest_keys.accepted(&code.secret());
        let mut tx = self.database.begin().await?;
        let session = Session::lock(&mut tx, &session).await?;
        let Some(pending) = tx
            .oidc()
            .lock_pending_device_authorization(&digests)
            .await?
        else {
            return Ok(DeviceLookup::Unknown);
        };
        match decision {
            DeviceDecision::Deny => tx.oidc().deny_device_authorization(pending.id).await?,
            DeviceDecision::Approve => {
                ClientPolicy::share(&mut tx, &pending.client_id)
                    .await?
                    .allows_device(&pending.scopes, &pending.audience)
                    .map_err(|refusal| refusal.reject(OAuthCode::InvalidRequest))?;
                self.approve_device(&mut tx, context, &session, &pending.client_id, &pending)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(DeviceLookup::Found(pending.client_display_name))
    }

    /// Approves `pending` for `client` as `session`'s account, inside `tx`: saves
    /// and records the consent, creates the device's session, which lasts no
    /// longer than the device code until the device collects its tokens, and
    /// records the approval.
    async fn approve_device(
        &self,
        tx: &mut DatabaseTransaction<'_>,
        context: RequestContext,
        session: &StoredSession,
        client: &ClientId,
        pending: &PendingDeviceAuthorization,
    ) -> Result<(), OAuthError> {
        tx.oidc()
            .save_consent(session.user_id, client, &pending.audience, &pending.scopes)
            .await?;
        let device = tx
            .sessions()
            .insert_for_device(SessionId::new(), session.id, client, pending.expires_at)
            .await?
            .ok_or(SessionError::Missing)?;
        tx.oidc()
            .approve_device_authorization(pending.id, session.user_id, device.id)
            .await?;
        let events = [
            DomainEvent::ConsentGranted {
                client_id: client.clone(),
                scopes: pending.scopes.clone(),
            },
            DomainEvent::DeviceAuthorized {
                client_id: client.clone(),
                session_id: device.id,
            },
        ];
        for event in events {
            tx.record(&context.user(session), Some(session.user_id), event)
                .await?;
        }
        Ok(())
    }

    /// Counts one lookup by user code against the browser's IP address and its
    /// `session`, and returns whether both budgets still allowed it.
    async fn spend_lookup(
        &self,
        context: RequestContext,
        session: &StoredSession,
    ) -> Result<bool, OAuthError> {
        let address = LOOKUPS.spend(&self.database, context.client_ip).await?;
        let session = LOOKUPS.spend(&self.database, session.id).await?;
        Ok(address && session)
    }

    /// Answers one poll of the authenticated client `client_id`'s device: the
    /// tokens once the user approved, or why not yet. Every poll is recorded, so
    /// one that comes before the interval has passed slows the device down, and a
    /// device code collects its tokens once. A poll that answers an error is
    /// recorded all the same: it is what slows the next one down.
    ///
    /// It locks the account that approved before the authorization, as a code
    /// exchange locks its own, so a poll never waits for an account while it
    /// holds the authorization that account's decision waits for.
    pub(super) async fn redeem_device(
        &self,
        request: DeviceRedemption,
        client_id: &ClientId,
        context: RequestContext,
    ) -> Result<TokenResponse, OAuthError> {
        let opaque = OpaqueToken::parse(TokenPrefix::DeviceCode, &request.device_code)?;
        let digests = self.accepted_digests(&opaque);
        let mut tx = self.database.begin().await?;
        let account = tx
            .oidc()
            .device_authorization_user(opaque.id, &digests)
            .await?;
        if let Some(user) = account {
            tx.users().find_by_id_for_update(user).await?;
        }
        let polled = tx
            .oidc()
            .poll_device_authorization(opaque.id, &digests, SLOW_DOWN_SECONDS)
            .await?
            .filter(|polled| &polled.client_id == client_id)
            .ok_or_else(|| {
                OAuthCode::InvalidGrant
                    .reject("device_code is unknown, or was issued to another client")
            })?;
        Scopes::same_target(request.resource.as_deref(), &polled.audience)?;
        match Self::poll_outcome(&polled, account.is_some()) {
            Ok((user_id, session_id)) => {
                self.issue_device_grant(tx, polled, user_id, session_id, context)
                    .await
            }
            Err(error) => {
                tx.commit().await?;
                Err(error)
            }
        }
    }

    /// Issues the tokens of the approved device authorization `polled`, locked in
    /// `tx` with the account `user_id` that approved it, to the device now
    /// polling from `context`, and commits. The account, the device's session
    /// `session_id`, the client's registration and the consent must all still
    /// allow it: the consent is read again under the account lock, as a code
    /// exchange reads it, since it may have been withdrawn since the approval
    /// saved it. The session lasts as long as a browser's from now on.
    async fn issue_device_grant(
        &self,
        mut tx: DatabaseTransaction<'_>,
        polled: PolledDeviceAuthorization,
        user_id: UserId,
        session_id: SessionId,
        context: RequestContext,
    ) -> Result<TokenResponse, OAuthError> {
        let user = tx
            .users()
            .find_active_by_id_for_update(user_id)
            .await?
            .ok_or_else(|| OAuthCode::InvalidGrant.reject(ACCOUNT_DISABLED))?;
        let ends = Utc::now() + Duration::days(i64::from(SESSION_LIFETIME_DAYS));
        let (address, family) = (context.client_ip, context.user_agent_family);
        let session = tx
            .sessions()
            .start_device(session_id, ends, address, family)
            .await?
            .ok_or_else(|| OAuthCode::InvalidGrant.reject(SESSION_ENDED))?;
        let policy = ClientPolicy::share(&mut tx, &polled.client_id).await?;
        policy
            .allows_device(&polled.scopes, &polled.audience)
            .map_err(|refusal| refusal.reject(OAuthCode::InvalidGrant))?;
        let client = policy.client();
        if !client.trusted
            && !tx
                .oidc()
                .consented(user.id, &client.id, &polled.audience, &polled.scopes)
                .await?
        {
            return Err(OAuthCode::InvalidGrant.reject(CONSENT_WITHDRAWN));
        }
        tx.oidc().redeem_device_authorization(polled.id).await?;
        tx.oidc()
            .record_session_client(session.id, &client.id)
            .await?;
        let email = tx.oidc().primary_email(user.id).await?;
        let scopes = Scopes::from_stored(&polled.scopes)?;
        let authentication = SessionAuthentication::from(&session);
        let tokens = TokenGrant {
            client: &client.id,
            audience: &polled.audience,
            scopes: &scopes,
            nonce: None,
            session: session.id,
            authentication: &authentication,
        }
        .issue(self, &user, email.as_deref())?;
        let family = InsertRefreshFamily {
            id: TokenId::new(),
            user_id: user.id,
            client_id: client.id.to_string(),
            session_id: session.id,
            authorization_code_id: None,
            audience: polled.audience,
            scopes: polled.scopes,
            authentication,
        };
        let (family_id, refresh_token) = self
            .create_refresh(&mut tx, client, &scopes, family)
            .await?
            .unzip();
        tx.record(
            &context.client(&client.id),
            Some(user.id),
            DomainEvent::GrantIssued {
                client_id: client.id.clone(),
                family_id,
                scopes: scopes.into_stored(),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(TokenResponse {
            refresh_token,
            ..tokens
        })
    }

    /// What a poll that found `polled` answers: who approved it and the session
    /// they created, when its tokens may be issued now, or the error telling the
    /// device what to do next (RFC 8628 §3.5). An approval made since the poll
    /// read who approved, so before it could lock their account (`locked` is
    /// false), reads as still pending: the device's next poll collects it.
    fn poll_outcome(
        polled: &PolledDeviceAuthorization,
        locked: bool,
    ) -> Result<(UserId, SessionId), OAuthError> {
        match polled.state {
            DeviceAuthorizationState::Redeemed => {
                Err(OAuthCode::InvalidGrant.reject("device_code was already used"))
            }
            DeviceAuthorizationState::Denied => {
                Err(OAuthCode::AccessDenied.reject("the user refused the request"))
            }
            DeviceAuthorizationState::Pending | DeviceAuthorizationState::Approved { .. }
                if polled.expired =>
            {
                Err(OAuthCode::ExpiredToken
                    .reject("device_code expired; start a new device authorization"))
            }
            DeviceAuthorizationState::Pending if polled.early => Err(OAuthCode::SlowDown.reject(
                format!("polled too soon; wait {SLOW_DOWN_SECONDS} seconds longer between polls"),
            )),
            DeviceAuthorizationState::Approved {
                user_id,
                session_id,
            } if locked => Ok((user_id, session_id)),
            DeviceAuthorizationState::Pending | DeviceAuthorizationState::Approved { .. } => {
                Err(OAuthCode::AuthorizationPending
                    .reject("the user has not decided yet; poll again after the interval"))
            }
        }
    }
}

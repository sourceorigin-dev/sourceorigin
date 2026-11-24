//! Authorization requests, pushed or from the browser, login/consent continuation
//! and the consent decision.

/// The consent decision, and the code an approved authorization issues.
mod consent;
/// Authorization request parameters and their validation.
mod parameters;
/// Pushed authorization requests (RFC 9126) and claiming them.
mod pushed;
/// Whether a session meets a pending authorization's sign-in requirements.
mod requirements;

pub(crate) use consent::AuthorizationDecision;
use consent::ConsentDecision;
use parameters::AuthorizationParameters;

use super::{
    OAuthCode, OAuthError, OidcApplication, OpaqueToken, Reject, TokenPrefix, policy::ClientPolicy,
};
use crate::{
    routes,
    security::{RequestContext, Session},
};
use axum::{
    body::Bytes,
    extract::{
        Query, RawForm, State,
        rejection::{QueryRejection, RawFormRejection},
    },
    http::HeaderMap,
    response::Redirect,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sourceorigin_common::{ClientId, RecoveryId, UserId, protocol::claims::Acr};
use sourceorigin_repository::StoredAuthorizationRequest;
use sourceorigin_telemetry::analytics::{Consent, Product};
use sourceorigin_ui::{Checkbox, LoginQuery};
use url::Url;

/// Rejection of a pending authorization that cannot be found.
const UNKNOWN_TRANSACTION: &str = "transaction is unknown, expired or already used";
/// Rejection of a `prompt=none` request without a suitable sign-in.
const SILENT_LOGIN_REQUIRED: &str = "prompt=none was requested, but the user must sign in first";

/// An authorization request's parameters as the browser sent them, form-encoded,
/// from the query or a form post. They are read in two steps: a pushed request's
/// reference first, so no other parameter can fail a request that carries one
/// (RFC 9126 §4), then every parameter of a request sent through the browser.
struct AuthorizationRequest(Bytes);

impl AuthorizationRequest {
    /// What a rejection of a request that cannot be read calls it.
    const NAME: &str = "authorization request";

    /// The request's parameters; `RawForm` reads the query string of `GET`
    /// requests.
    fn new(form: Result<RawForm, RawFormRejection>) -> Result<Self, OAuthError> {
        form.map(|RawForm(form)| Self(form))
            .map_err(|error| OAuthError::malformed(Self::NAME, error))
    }

    /// The parameters `T` names, ignoring the others.
    fn read<T: DeserializeOwned>(&self) -> Result<T, OAuthError> {
        serde_urlencoded::from_bytes(&self.0)
            .map_err(|error| OAuthError::malformed(Self::NAME, error))
    }
}

/// The parameters of a request continuing a pushed one: only these count.
#[derive(Deserialize)]
struct PushedReference {
    /// The client that pushed the request.
    client_id: Option<String>,
    /// The pushed request's reference.
    request_uri: Option<String>,
}

/// A pending authorization, by its opaque token: the one the browser returns with
/// after signing in, or the consent page shows.
#[derive(Serialize, Deserialize)]
pub(crate) struct TransactionQuery {
    /// Opaque pending authorization token.
    pub(crate) transaction: String,
}

impl sourceorigin_ui::Query for TransactionQuery {}

/// Whether the browser answered the question a sign-in for an application asks
/// while the account has a recovery that can still be undone: was it the
/// owner's? Only the sign-in page's form answers it, so no link can skip it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryQuestion {
    /// Not yet: a sign-in for the authorization asks it before going on.
    Unanswered,
    /// The browser said yes to this recovery's question, and goes on. A question
    /// about another recovery, as another account's, is still asked.
    Answered(RecoveryId),
}

/// A pending authorization, as the sign-in page continuing it sees it.
pub(crate) struct AuthorizationView {
    /// The class the sign-in must reach, if the application asked for one.
    pub(crate) target_acr: Option<Acr>,
    /// Where the application receives the browser: the registered redirect URI the
    /// request named.
    pub(crate) redirect_uri: Url,
}

sourceorigin_telemetry::signal! {
    /// A step an authorization request passed: how many requests of each app
    /// reach each step, so where people stop signing in shows, never whose.
    pub(crate) product struct AuthorizationStep in "authorization_steps", kept 390 days, ordered by (client, step) {
        /// The app.
        pub(crate) client: ClientId,
        /// `requested`, the app sent the browser here; `sign_in`, the sign-in page
        /// showed; `consent`, the consent page showed; `authorized`, the app got
        /// its code; or `denied`, the person declined.
        pub(crate) step: &'static str,
    }
}

impl OidcApplication {
    /// The row that a request of `client` reached `step`, with the consent of
    /// the person behind `context`, `user` once signed in, if they give it.
    async fn step(
        &self,
        context: &RequestContext,
        user: Option<UserId>,
        client: &ClientId,
        step: &'static str,
    ) -> Option<(AuthorizationStep, Consent)> {
        let consent = context.consent_of(&self.database, user).await?;
        let client = client.clone();
        Some((AuthorizationStep { client, step }, consent))
    }

    /// Records that a request of `client` reached `step`; see [`Self::step`].
    async fn reached(
        &self,
        context: &RequestContext,
        user: Option<UserId>,
        client: &ClientId,
        step: &'static str,
    ) {
        if let Some((row, consent)) = self.step(context, user, client, step).await {
            row.record(consent);
        }
    }
}

impl OidcApplication {
    /// Starts an authorization by `GET` query or form `POST`: validates it, stores it
    /// as a pending transaction, then sends the browser to login, consent or the
    /// client. A `request_uri` continues a pushed request instead, whatever the
    /// other parameters say.
    pub(super) async fn authorize(
        State(app): State<Self>,
        context: RequestContext,
        headers: HeaderMap,
        form: Result<RawForm, RawFormRejection>,
    ) -> Result<Redirect, OAuthError> {
        let next = app
            .start_authorization(context, &headers, &AuthorizationRequest::new(form)?)
            .await?;
        Ok(Redirect::to(next.as_str()))
    }

    /// Resumes an authorization after the browser signs in.
    pub(super) async fn resume(
        State(app): State<Self>,
        context: RequestContext,
        headers: HeaderMap,
        query: Result<Query<TransactionQuery>, QueryRejection>,
    ) -> Result<Redirect, OAuthError> {
        let Query(q) = query.map_err(|error| OAuthError::malformed("resume request", error))?;
        let next = app
            .continue_authorization(
                context,
                &headers,
                q.transaction,
                RecoveryQuestion::Unanswered,
            )
            .await?;
        Ok(Redirect::to(next.as_str()))
    }

    /// Where [`Self::authorize`] sends the browser for `request`. A rejection goes
    /// back to the client once its redirect URI is verified.
    async fn start_authorization(
        &self,
        context: RequestContext,
        headers: &HeaderMap,
        request: &AuthorizationRequest,
    ) -> Result<Url, OAuthError> {
        let pushed: PushedReference = request.read()?;
        if let Some(request_uri) = &pushed.request_uri {
            let client_id = pushed.client_id.as_deref().unwrap_or_default();
            return self
                .claim_pushed(context, headers, client_id, request_uri)
                .await;
        }
        let q: AuthorizationParameters = request.read()?;
        let client_id = q.client_id.as_deref().unwrap_or_default();
        let client = self
            .enabled_client(client_id, OAuthCode::InvalidRequest)
            .await?;
        let redirect = q.redirect_uri(&client)?;
        q.state()?;
        let client_redirect = self.client_redirect(&redirect, q.state.as_deref());
        let back = |error: OAuthError| error.redirected(&client_redirect);
        if client.require_pushed_authorization_requests {
            return Err(back(OAuthCode::UnauthorizedClient.reject(
                "this client must push its authorization requests and send their request_uri",
            )));
        }
        let validated = q.validate(self, &client, false).map_err(back)?;
        let request = &validated.request;
        let session = Session::resolve(&self.database, context, headers).await?;
        let requires_login = session.as_ref().is_none_or(|session| {
            !Self::session_satisfies(
                session,
                request.interaction.forces_login(),
                request.max_age,
                request.hinted_user_id,
                request.target_acr,
                None,
            )
        });
        if request.interaction.is_silent() && requires_login {
            return Err(back(OAuthCode::LoginRequired.reject(SILENT_LOGIN_REQUIRED)));
        }
        let target_acr = request.target_acr;
        let transaction = validated.store(self).await?;
        let user = session.as_ref().map(|session| session.user_id);
        self.reached(&context, user, &client.id, "requested").await;
        if requires_login {
            self.reached(&context, user, &client.id, "sign_in").await;
            self.sign_in_page(transaction, target_acr)
        } else {
            self.continue_authorization(context, headers, transaction, RecoveryQuestion::Unanswered)
                .await
        }
    }

    /// The pending authorization `transaction`, as the sign-in page continuing it sees
    /// it: what it may offer and where its forms may lead. The token is all it takes:
    /// the browser is not signed in yet.
    pub(crate) async fn authorization(
        &self,
        transaction: &str,
    ) -> Result<AuthorizationView, OAuthError> {
        let request = self.find_authorization_request(transaction).await?;
        Ok(AuthorizationView {
            target_acr: request.target_acr,
            redirect_uri: Url::parse(&request.redirect_uri)?,
        })
    }

    /// Finds a live pending authorization by its opaque token, while its client
    /// still allows it. Every page and redirect continuing an authorization starts
    /// here, so none leads the browser to a redirect URI, or for a request, that
    /// the client no longer allows: such a request explains itself on auth's page
    /// instead.
    async fn find_authorization_request(
        &self,
        value: &str,
    ) -> Result<StoredAuthorizationRequest, OAuthError> {
        let opaque = OpaqueToken::parse(TokenPrefix::AuthorizationRequest, value)?;
        let digests = self.accepted_digests(&opaque);
        let request = self
            .database
            .oidc()
            .find_authorization_request(opaque.id, &digests)
            .await?
            .ok_or_else(|| OAuthCode::InvalidRequest.reject(UNKNOWN_TRANSACTION))?;
        ClientPolicy::read(&self.database, &request.client_id)
            .await?
            .allows(&request)
            .map_err(|refusal| refusal.reject(OAuthCode::InvalidRequest))?;
        Ok(request)
    }

    /// The sign-in page continuing the pending authorization `transaction`. A target
    /// `acr` is named too, so the page offers only methods reaching it.
    fn sign_in_page(
        &self,
        transaction: String,
        target_acr: Option<Acr>,
    ) -> Result<Url, OAuthError> {
        let query = LoginQuery {
            transaction: Some(transaction),
            acr: target_acr,
            ..LoginQuery::default()
        };
        self.page(routes::page::LOGIN, &query)
    }

    /// Where the browser goes on from a stored authorization: to sign-in if the session
    /// falls short, straight to the client if consent is already given, otherwise to
    /// consent. `prompt=none` returns an error to the client instead of showing a page.
    /// Looking the session up records its activity, so single sign-on keeps it current.
    /// The sign-in pages call it once the browser signed in, and every other sign-in,
    /// a provider's or a restore, goes on through it by resuming the authorization.
    /// A sign-in for this authorization first goes to the sign-in page's question
    /// about the account's recovery that can still be undone, until `recovery` says
    /// the browser answered it.
    pub(crate) async fn continue_authorization(
        &self,
        context: RequestContext,
        headers: &HeaderMap,
        transaction: String,
        recovery: RecoveryQuestion,
    ) -> Result<Url, OAuthError> {
        let request = self.find_authorization_request(&transaction).await?;
        let silent = request.interaction.is_silent();
        let redirect = Url::parse(&request.redirect_uri)?;
        let client_redirect = self.client_redirect(&redirect, request.client_state.as_deref());
        let back =
            |code: OAuthCode, description| code.reject(description).redirected(&client_redirect);
        let session = Session::resolve(&self.database, context, headers).await?;
        if session
            .as_ref()
            .is_some_and(|session| Self::falls_short_after_signing_in(&request, session))
        {
            return Err(back(
                OAuthCode::UnmetAuthenticationRequirements,
                "this account cannot meet the requested acr_values for now",
            ));
        }
        let session = session
            .filter(|session| Self::require_authorization_session(&request, session).is_ok());
        let Some(session) = session else {
            if silent {
                return Err(back(OAuthCode::LoginRequired, SILENT_LOGIN_REQUIRED));
            }
            self.reached(&context, None, &request.client_id, "sign_in")
                .await;
            return self.sign_in_page(transaction, request.target_acr);
        };
        if !silent && Self::signed_in_for(&request, &session) {
            let mut tx = self.database.begin().await?;
            let undoable = Self::undoable_recovery(&mut tx, session.user_id).await?;
            tx.commit().await?;
            if let Some(id) = undoable
                && recovery != RecoveryQuestion::Answered(id)
            {
                let query = LoginQuery {
                    transaction: Some(transaction),
                    recovery: Checkbox(true),
                    ..LoginQuery::default()
                };
                return self.page(routes::page::LOGIN, &query);
            }
        }
        if self.can_authorize(&request, &session).await? {
            let input = AuthorizationDecision {
                transaction,
                decision: ConsentDecision::Approve,
            };
            return self.decide(context, headers, input, false).await;
        }
        if silent {
            return Err(back(
                OAuthCode::ConsentRequired,
                "prompt=none was requested, but the user must consent first",
            ));
        }
        self.reached(
            &context,
            Some(session.user_id),
            &request.client_id,
            "consent",
        )
        .await;
        self.page(routes::page::CONSENT, &TransactionQuery { transaction })
    }
}

/// Prompt, `max_age` and consent decision parsing.
#[cfg(test)]
mod tests;

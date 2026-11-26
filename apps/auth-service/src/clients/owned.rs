//! The clients an account owns: the policy owners register and change them
//! under, and the [`AccountService`] operations that apply it, for the owner
//! only. The policy lives in the input types rather than in callers: an owner
//! never chooses a client's ID, which auth generates, or its trust, so its
//! clients always show the consent screen, scopes are limited to the standard
//! [`Scope`]s, and names to those without a reserved brand. [`OperatorSettings`]
//! validate the same settings for an operator, who also chooses any scope, any
//! name and a client's trust.

use super::{ClientRegistry, ClientRegistryError, MAX_DAILY_REGISTRATIONS, MAX_OWNED_CLIENTS};
use crate::{
    GracePeriod,
    account::{AccountError, AccountService, Owner},
    security::ApiError,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sourceorigin_common::{
    ClientId,
    admin::{ClientResource, SecretLifetime},
    clients::{
        AllowedScopes, ClientField, ClientKind, ClientOwner, ClientRegistration, PostLogoutUris,
        RedirectUri, RedirectUris,
    },
    protocol::oauth::{GrantType, Scope},
};
use sourceorigin_crypto::{ClientSecret, Secret};
use sourceorigin_repository::{DatabaseTransaction, StoredOidcClient};
use std::error::Error;
use thiserror::Error;

/// The settings an owner chooses, validated when a client is registered or
/// changed with them.
pub(crate) struct ClientSettings {
    /// The name consent screens show.
    pub(crate) display_name: String,
    /// Authorization callbacks, matched exactly.
    pub(crate) redirect_uris: Vec<String>,
    /// Pages the browser may return to after logout.
    pub(crate) post_logout_uris: Vec<String>,
    /// The scopes auth defines that the client may request.
    pub(crate) allowed_scopes: Vec<Scope>,
    /// The grants the client may use at the token endpoint.
    pub(crate) grant_types: Vec<GrantType>,
    /// Whether the client must push its authorization requests (RFC 9126).
    pub(crate) require_pushed_authorization_requests: bool,
    /// Where auth posts logout tokens, if anywhere.
    pub(crate) backchannel_logout_uri: Option<String>,
}

/// The settings an operator chooses for any client: an owner's, with any name
/// and the client's trust.
pub(crate) struct OperatorSettings {
    /// What an owner would choose.
    pub(crate) settings: ClientSettings,
    /// Whether the client skips the consent screen.
    pub(crate) trusted: bool,
}

/// A client an owner registers.
pub(crate) struct NewClient {
    /// Public, or confidential with a secret auth generates.
    pub(crate) kind: ClientKind,
    /// Everything else.
    pub(crate) settings: ClientSettings,
}

/// A change an owner makes: each given setting replaces the stored one, and any
/// change needs a passkey confirmation. The default changes nothing.
#[derive(Default)]
pub(crate) struct ClientPatch {
    /// A new consent-screen name.
    pub(crate) display_name: Option<String>,
    /// New authorization callbacks.
    pub(crate) redirect_uris: Option<Vec<String>>,
    /// New post-logout pages.
    pub(crate) post_logout_uris: Option<Vec<String>>,
    /// New scopes.
    pub(crate) allowed_scopes: Option<Vec<Scope>>,
    /// New grant types.
    pub(crate) grant_types: Option<Vec<GrantType>>,
    /// Whether authorization requests must be pushed.
    pub(crate) require_pushed_authorization_requests: Option<bool>,
    /// Where logout tokens are posted: `Some(None)` stops posting them.
    #[expect(
        clippy::option_option,
        reason = "A patch keeps the URI, removes it or sets it: the outer `None` keeps it."
    )]
    pub(crate) backchannel_logout_uri: Option<Option<String>>,
}

/// One of an owner's live clients, with its owner. Only
/// [`AccountService::owned_client`] builds one, once it checked the client is the
/// owner's, so the operations on it never check ownership again: it never
/// changes, and the registry refuses a client deleted since.
pub(crate) struct OwnedClient {
    /// The owner.
    owner: Owner,
    /// The client, as it was when found.
    client: OwnedClientView,
}

/// One of an owner's clients as its owner sees it: its settings, status and last
/// use, never its trust, audiences or owner, which only operators manage.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct OwnedClientView {
    /// Client identifier.
    pub(crate) id: ClientId,
    /// The name consent screens show.
    pub(crate) display_name: String,
    /// Public, or confidential with secrets.
    pub(crate) kind: ClientKind,
    /// Authorization callbacks, matched exactly.
    pub(crate) redirect_uris: Vec<String>,
    /// Pages the browser may return to after logout.
    pub(crate) post_logout_uris: Vec<String>,
    /// The scopes auth defines that the client may request.
    pub(crate) allowed_scopes: Vec<String>,
    /// The resources it may request tokens for, as their owners granted, each
    /// with the scopes of the resource it may request.
    pub(crate) resources: Vec<ClientResource>,
    /// The grants the client may use at the token endpoint.
    pub(crate) grant_types: Vec<GrantType>,
    /// Whether the client must push its authorization requests (RFC 9126).
    pub(crate) require_pushed_authorization_requests: bool,
    /// Where logout tokens are posted, if anywhere.
    pub(crate) backchannel_logout_uri: Option<String>,
    /// Whether it signs users in, as its owner or an operator chose.
    pub(crate) enabled: bool,
    /// When an operator suspended it, which keeps it disabled.
    pub(crate) suspended_at: Option<DateTime<Utc>>,
    /// When a token was last issued to it, to within an hour.
    pub(crate) last_used_at: Option<DateTime<Utc>>,
}

/// Why an owner's request about their clients was refused.
#[derive(Debug, Error)]
pub(crate) enum OwnedClientError {
    /// The account owns no live client with this ID: another account's and a
    /// deleted one's included, so probing IDs reveals nothing.
    #[error("client not found")]
    NotFound,
    /// A setting is invalid.
    #[error(transparent)]
    InvalidSetting(#[from] InvalidSetting),
    /// The registry refused the change, or failed.
    #[error(transparent)]
    Registry(#[from] ClientRegistryError),
}

/// A setting an owner gave that is invalid, with the field it was given for, so
/// a form can show the message next to it.
#[derive(Debug, Error)]
#[error("{field}: {source}")]
pub(crate) struct InvalidSetting {
    /// The rejected setting.
    field: ClientField,
    /// What is wrong with it, from the type that validates it.
    source: Box<dyn Error + Send + Sync>,
}

impl ClientSettings {
    /// Validates every setting, naming the first one that is invalid, and builds
    /// the registration of an untrusted client `id` of `kind`. Each URI is a
    /// [`RedirectUri`], as the `client` command parses it, and the name must not
    /// contain a reserved brand, which only operators' clients use.
    fn registration(
        self,
        id: ClientId,
        kind: ClientKind,
    ) -> Result<ClientRegistration, InvalidSetting> {
        let registration = self.validated(id, kind, false)?;
        registration
            .display_name
            .ensure_unreserved()
            .map_err(InvalidSetting::with(ClientField::DisplayName))?;
        Ok(registration)
    }

    /// Validates every setting, and builds the registration of client `id` of
    /// `kind`, `trusted` or not.
    fn validated(
        self,
        id: ClientId,
        kind: ClientKind,
        trusted: bool,
    ) -> Result<ClientRegistration, InvalidSetting> {
        Ok(ClientRegistration {
            id,
            display_name: self
                .display_name
                .parse()
                .map_err(InvalidSetting::with(ClientField::DisplayName))?,
            kind,
            redirect_uris: RedirectUris::new(Self::uris(
                &ClientField::RedirectUris,
                &self.redirect_uris,
            )?)
            .map_err(InvalidSetting::with(ClientField::RedirectUris))?,
            post_logout_uris: PostLogoutUris::new(Self::uris(
                &ClientField::PostLogoutUris,
                &self.post_logout_uris,
            )?)
            .map_err(InvalidSetting::with(ClientField::PostLogoutUris))?,
            allowed_scopes: AllowedScopes::new(self.allowed_scopes)
                .map_err(InvalidSetting::with(ClientField::AllowedScopes))?,
            grant_types: kind
                .grant_types(self.grant_types)
                .map_err(InvalidSetting::with(ClientField::GrantTypes))?,
            trusted,
            require_pushed_authorization_requests: self.require_pushed_authorization_requests,
            backchannel_logout_uri: self
                .backchannel_logout_uri
                .as_deref()
                .map(str::parse)
                .transpose()
                .map_err(InvalidSetting::with(ClientField::BackchannelLogoutUri))?,
        })
    }

    /// Parses the URIs of `field`.
    fn uris(field: &ClientField, uris: &[String]) -> Result<Vec<RedirectUri>, InvalidSetting> {
        uris.iter()
            .map(|uri| uri.parse().map_err(InvalidSetting::with(field.clone())))
            .collect()
    }
}

impl OperatorSettings {
    /// Validates every setting, naming the first one that is invalid, and builds
    /// the registration of client `id` of `kind`.
    pub(crate) fn registration(
        self,
        id: ClientId,
        kind: ClientKind,
    ) -> Result<ClientRegistration, InvalidSetting> {
        self.settings.validated(id, kind, self.trusted)
    }
}

impl From<ClientSettings> for ClientPatch {
    /// Replaces every setting an owner chooses with `settings`', as a form showing
    /// them all saves them.
    fn from(settings: ClientSettings) -> Self {
        Self {
            display_name: Some(settings.display_name),
            redirect_uris: Some(settings.redirect_uris),
            post_logout_uris: Some(settings.post_logout_uris),
            allowed_scopes: Some(settings.allowed_scopes),
            grant_types: Some(settings.grant_types),
            require_pushed_authorization_requests: Some(
                settings.require_pushed_authorization_requests,
            ),
            backchannel_logout_uri: Some(settings.backchannel_logout_uri),
        }
    }
}

impl ClientPatch {
    /// The registration of `current` with the given settings replaced. It is
    /// validated as a new one is, so a change cannot do what registration may not.
    fn apply(self, current: &StoredOidcClient) -> Result<ClientRegistration, InvalidSetting> {
        ClientSettings {
            display_name: self
                .display_name
                .unwrap_or_else(|| current.display_name.clone()),
            redirect_uris: self
                .redirect_uris
                .unwrap_or_else(|| current.redirect_uris.clone()),
            post_logout_uris: self
                .post_logout_uris
                .unwrap_or_else(|| current.post_logout_uris.clone()),
            allowed_scopes: self.allowed_scopes.unwrap_or_else(|| {
                (current.allowed_scopes.iter())
                    .filter_map(|scope| scope.parse().ok())
                    .collect()
            }),
            grant_types: self
                .grant_types
                .unwrap_or_else(|| current.grant_types.clone()),
            require_pushed_authorization_requests: self
                .require_pushed_authorization_requests
                .unwrap_or(current.require_pushed_authorization_requests),
            backchannel_logout_uri: self
                .backchannel_logout_uri
                .unwrap_or_else(|| current.backchannel_logout_uri.clone()),
        }
        .registration(current.id.clone(), current.kind)
    }
}

impl InvalidSetting {
    /// Rejects `field` for `reason`.
    fn new(field: ClientField, reason: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self {
            field,
            source: reason.into(),
        }
    }

    /// Rejects `field` for the error the mapped function receives.
    fn with<E: Error + Send + Sync + 'static>(field: ClientField) -> impl Fn(E) -> Self {
        move |error| Self::new(field.clone(), error)
    }

    /// The rejected setting.
    pub(crate) const fn field(&self) -> &ClientField {
        &self.field
    }

    /// What is wrong with the setting, without the field's name: what a form
    /// shows next to it.
    pub(crate) fn reason(&self) -> &(dyn Error + Send + Sync) {
        &*self.source
    }
}

impl OwnedClient {
    /// The client as its owner sees it, as it was when found.
    pub(crate) const fn client(&self) -> &OwnedClientView {
        &self.client
    }
}

impl From<StoredOidcClient> for OwnedClientView {
    /// Keeps what its owner sees.
    fn from(client: StoredOidcClient) -> Self {
        Self {
            id: client.id,
            display_name: client.display_name,
            kind: client.kind,
            redirect_uris: client.redirect_uris,
            post_logout_uris: client.post_logout_uris,
            allowed_scopes: client.allowed_scopes,
            resources: client.resources,
            grant_types: client.grant_types,
            require_pushed_authorization_requests: client.require_pushed_authorization_requests,
            backchannel_logout_uri: client.backchannel_logout_uri,
            enabled: client.enabled,
            suspended_at: client.suspended_at,
            last_used_at: client.last_used_at,
        }
    }
}

/// The clients only their owner registers and changes. Administrators manage
/// every client through `AdminService` instead.
impl AccountService {
    /// Lists the owner's live clients. Clients operators manage are never shown.
    pub(crate) async fn clients(
        &self,
        owner: &Owner,
    ) -> Result<Vec<OwnedClientView>, AccountError> {
        let clients = self
            .database
            .oidc()
            .list_owned_clients(owner.user())
            .await?;
        Ok(clients.into_iter().map(OwnedClientView::from).collect())
    }

    /// The owner's live client `id`, as it is now. Any other ID, a deleted
    /// client's or another account's included, is not found, so probing IDs
    /// reveals nothing. Every operation on one client starts here.
    pub(crate) async fn owned_client(
        &self,
        owner: Owner,
        id: &str,
    ) -> Result<OwnedClient, AccountError> {
        let client = self
            .database
            .oidc()
            .find_client(id)
            .await?
            .filter(|client| client.owner_id == Some(owner.user()) && client.deleted_at.is_none())
            .ok_or(OwnedClientError::NotFound)?;
        Ok(OwnedClient {
            owner,
            client: client.into(),
        })
    }

    /// The lifetimes of an owner's client's secrets, oldest first, never their
    /// values.
    pub(crate) async fn client_secrets(
        &self,
        owned: &OwnedClient,
    ) -> Result<Vec<SecretLifetime>, AccountError> {
        let secrets = self
            .database
            .oidc()
            .client_secrets(&owned.client.id)
            .await?;
        Ok(secrets.into_iter().map(Into::into).collect())
    }

    /// Runs `change` on the client registry in one transaction, attributed to
    /// `owner`, once they confirmed with a passkey under the account lock, and
    /// commits it. The registry records each change's event itself.
    async fn change_clients<T, E>(
        &self,
        owner: &Owner,
        change: impl AsyncFnOnce(&mut ClientRegistry<'_, '_>) -> Result<T, E>,
    ) -> Result<T, AccountError>
    where
        AccountError: From<E>,
    {
        let mut tx = self.database.begin().await?;
        owner.lock_confirmed(&mut tx).await?;
        let value = change(&mut ClientRegistry::new(&mut tx, owner.context())).await?;
        tx.commit().await?;
        Ok(value)
    }

    /// Registers a client the owner manages, with an ID auth generates, once they
    /// confirmed with a passkey, and returns it with a confidential client's
    /// secret, which nothing shows again. Invalid settings are refused before the
    /// confirmation is checked. Fails once the account manages
    /// [`MAX_OWNED_CLIENTS`](super::MAX_OWNED_CLIENTS) clients.
    pub(crate) async fn create_client(
        &self,
        owner: &Owner,
        new: NewClient,
    ) -> Result<(OwnedClientView, Option<ClientSecret>), AccountError> {
        let random = Secret::generate().map_err(ClientRegistryError::from)?;
        let registration = new
            .settings
            .registration(ClientId::generated(random.as_bytes()), new.kind)
            .map_err(OwnedClientError::from)?;
        self.change_clients(owner, async |clients| {
            let owned_by = ClientOwner::Account(owner.user());
            let secret = clients.create(owned_by, &registration, None).await?;
            let client = Self::read_client(clients.tx, &registration.id).await?;
            Ok::<_, AccountError>((client, secret))
        })
        .await
    }

    /// Changes the given settings of an owner's client, once they confirmed with
    /// a passkey, and returns it. Every change needs the confirmation, like every
    /// other change to a client: with a stolen session cookie, someone could
    /// otherwise add their own redirect URI and collect authorization codes, or
    /// widen what a client they control may ask for.
    pub(crate) async fn update_client(
        &self,
        owned: &OwnedClient,
        patch: ClientPatch,
    ) -> Result<OwnedClientView, AccountError> {
        let id = &owned.client.id;
        self.change_clients(&owned.owner, async |clients| {
            clients
                .update(id, |current| {
                    patch.apply(current).map_err(OwnedClientError::from)
                })
                .await?;
            Self::read_client(clients.tx, id).await
        })
        .await
    }

    /// Enables or disables an owner's client, once they confirmed with a passkey.
    /// Disabling keeps the client's refresh grants, so enabling it again revives
    /// every user's; a stolen session cookie must not be enough for that. A
    /// suspended client stays disabled.
    pub(crate) async fn set_client_enabled(
        &self,
        owned: &OwnedClient,
        enabled: bool,
    ) -> Result<(), AccountError> {
        self.change_clients(&owned.owner, async |clients| {
            clients.set_enabled(&owned.client.id, enabled).await
        })
        .await
    }

    /// Gives an owner's confidential client a new secret, once they
    /// confirmed with a passkey, and returns it, which nothing shows again.
    /// Earlier secrets keep working for `grace`.
    pub(crate) async fn rotate_client_secret(
        &self,
        owned: &OwnedClient,
        grace: GracePeriod,
    ) -> Result<ClientSecret, AccountError> {
        self.change_clients(&owned.owner, async |clients| {
            clients.rotate_secret(&owned.client.id, grace).await
        })
        .await
    }

    /// Deletes an owner's client for good, once they confirmed with a
    /// passkey: it stops working at once, and its ID is never used again.
    pub(crate) async fn delete_client(&self, owned: &OwnedClient) -> Result<(), AccountError> {
        self.change_clients(&owned.owner, async |clients| {
            clients.delete(&owned.client.id).await
        })
        .await
    }

    /// Reads back client `id` as the transaction changed it.
    async fn read_client(
        tx: &mut DatabaseTransaction<'_>,
        id: &ClientId,
    ) -> Result<OwnedClientView, AccountError> {
        Ok(tx
            .oidc()
            .find_client(id.as_str())
            .await?
            .map(OwnedClientView::from)
            .ok_or(OwnedClientError::NotFound)?)
    }
}

impl From<ClientRegistryError> for AccountError {
    /// A registry failure of one of the owner's clients, as [`OwnedClientError`]
    /// reports it.
    fn from(error: ClientRegistryError) -> Self {
        OwnedClientError::from(error).into()
    }
}

impl From<OwnedClientError> for ApiError {
    /// Answers as each failure's own mapping does.
    fn from(error: OwnedClientError) -> Self {
        match error {
            OwnedClientError::NotFound => Self::not_found("You have no client with this ID."),
            OwnedClientError::InvalidSetting(error) => error.into(),
            OwnedClientError::Registry(error) => error.into(),
        }
    }
}

impl From<InvalidSetting> for ApiError {
    /// Names the field before what is wrong with it.
    fn from(error: InvalidSetting) -> Self {
        Self::bad_request(error.to_string())
    }
}

impl From<ClientRegistryError> for ApiError {
    /// A deleted client is as unknown as one that never existed. Refused related
    /// origins, which only the dashboard's administrators cause, name what to fix.
    /// Other failures an owner cannot cause are internal.
    fn from(error: ClientRegistryError) -> Self {
        match error {
            ClientRegistryError::Unknown(_) | ClientRegistryError::Deleted(_) => {
                OwnedClientError::NotFound.into()
            }
            ClientRegistryError::Public(_) => {
                Self::bad_request("This client is public, so it has no secret to rotate.")
            }
            ClientRegistryError::QuotaExceeded => Self::conflict(format!(
                "You can have at most {MAX_OWNED_CLIENTS} clients. Delete one to register another."
            )),
            ClientRegistryError::DailyLimitExceeded => Self::conflict(format!(
                "You can register at most {MAX_DAILY_REGISTRATIONS} clients a day, deleted ones \
                 included. Try again tomorrow."
            )),
            ClientRegistryError::Suspended(_) => Self::forbidden(
                "This client was suspended by an administrator. Contact them to restore it.",
            ),
            error @ (ClientRegistryError::Untrusted(_)
            | ClientRegistryError::NotRedirectOrigin { .. }
            | ClientRegistryError::PublicSuffix(_)) => Self::bad_request(error.to_string()),
            error @ (ClientRegistryError::TooManyDomains(_)
            | ClientRegistryError::OriginApproved { .. }) => Self::conflict(error.to_string()),
            error @ ClientRegistryError::OriginNotApproved { .. } => {
                Self::not_found(error.to_string())
            }
            ClientRegistryError::Database(error) => error.into(),
            error @ (ClientRegistryError::Exists { .. }
            | ClientRegistryError::MissingSecret(_)
            | ClientRegistryError::Immutable(_)
            | ClientRegistryError::Crypto(_)) => Self::internal(error),
        }
    }
}

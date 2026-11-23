//! The account's email addresses: adding one sends a verification link, and a
//! verified address can become the primary one, which the OIDC `email` claim
//! reports. Adding, changing the primary address and removing one are sensitive,
//! since the addresses receive security notices, so they need a recent passkey
//! confirmation. Verifying needs only the link's token: possessing it proves
//! control of the mailbox. Administrators may list, change the primary address and
//! remove, but only the owner adds an address.
//!
//! The token is never stored, not even in the delivery that sends it: it is
//! derived from the address's ID with [`VerificationKeys`], once here to store its
//! digest, and again by the email sender. See `docs/dev/architecture/delivery-queue.md`.
use crate::{
    account::{AccountError, AccountService, Authorized, ConfirmedOwner, Owner, Subject, Viewer},
    config::{EmailConfig, LinkKeys},
    routes,
    security::{ApiError, RequestContext},
};
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sourceorigin_common::{
    EmailId, ErrorChain, Personal, Provider, UserId,
    email::NormalizedEmail,
    events::{Actor, DomainEvent, EventContext},
};
use sourceorigin_crypto::{Digest, Secret};
use sourceorigin_repository::{
    Database, DatabaseError, DatabaseTransaction, InsertUnverifiedEmail, InsertVerifiedEmail,
    PendingFederation, StoredEmail,
};
use std::{net::IpAddr, time::Duration as StdDuration};

/// Email failures and their browser response mapping.
mod error;
pub(crate) use error::EmailError;

/// How long a verification link works.
pub(crate) const VERIFICATION_LIFETIME: Duration = Duration::hours(24);
/// The most addresses an account may have, verified or not.
const MAX_EMAILS: usize = 10;
/// The window verification email limits count in.
const VERIFICATION_WINDOW: StdDuration = StdDuration::from_hours(1);
/// Verification emails one account may send per [`VERIFICATION_WINDOW`].
const VERIFICATIONS_PER_ACCOUNT: i32 = 5;
/// Verification emails one client IP may send per [`VERIFICATION_WINDOW`], across
/// accounts.
const VERIFICATIONS_PER_IP: i32 = 10;
/// Verification emails one address may receive per [`VERIFICATION_WINDOW`], across
/// accounts and clients.
const VERIFICATIONS_PER_ADDRESS: i32 = 3;

/// HMAC keys deriving each address's verification token from its ID. They are
/// derived from the token digest keys, so every replica and the email sender
/// derive the same token, a restart does not break links already sent, and a
/// change of token digest key does not break those still queued.
#[derive(Clone)]
pub(crate) struct VerificationKeys(LinkKeys);

impl VerificationKeys {
    /// Wraps keys reserved for verification tokens.
    pub(crate) const fn new(keys: LinkKeys) -> Self {
        Self(keys)
    }

    /// The token of new address `id`'s verification link, under the current key.
    pub(crate) fn token(&self, id: EmailId) -> Secret {
        self.0.token(id.as_uuid().as_bytes())
    }

    /// The token of address `id`'s verification link whose SHA-256 is the stored
    /// `digest`, under the key that derives it. `None` once no configured key
    /// does.
    pub(crate) fn rebuild(&self, id: EmailId, digest: &[u8]) -> Option<Secret> {
        self.0.rebuild(id.as_uuid().as_bytes(), digest)
    }
}

/// The account's addresses and their verification.
#[derive(Clone)]
pub(crate) struct EmailApplication {
    /// Addresses, sessions and events.
    database: Database,
    /// Whether email is sent, and the verification key.
    config: EmailConfig,
}

/// An address as the account sees it.
#[derive(Serialize)]
pub(crate) struct EmailView {
    /// Address ID.
    pub(crate) id: EmailId,
    /// The normalized address.
    pub(crate) address: String,
    /// Whether someone followed its verification link.
    pub(crate) verified: bool,
    /// Whether it is the primary address, which applications receive.
    pub(crate) primary: bool,
}

impl From<StoredEmail> for EmailView {
    /// Keeps what the account sees.
    fn from(email: StoredEmail) -> Self {
        Self {
            id: email.id,
            verified: email.verified_at.is_some(),
            address: email.normalized_email,
            primary: email.is_primary,
        }
    }
}

/// The body of [`routes::emails::LIST`] mutations.
#[derive(Deserialize)]
struct AddEmail {
    /// The address to add.
    email: String,
}

/// The body of [`routes::emails::VERIFY`].
#[derive(Deserialize)]
struct VerifyEmail {
    /// The token from the verification link.
    token: String,
}

/// The answer to a verification.
#[derive(Serialize)]
pub(crate) struct Verified {
    /// The verified address.
    pub(crate) address: String,
    /// Whether it became the primary address, being the account's first.
    pub(crate) primary: bool,
}

impl EmailApplication {
    /// Manages addresses in `database`, sending email as `config` says.
    pub(crate) fn new(database: Database, config: &EmailConfig) -> Self {
        Self {
            database,
            config: config.clone(),
        }
    }

    /// Returns the routes that add and verify addresses, with this application as
    /// state. Only the owner can add a channel that receives the account's email;
    /// listing, changing the primary address and removing are shared settings,
    /// which [`AccountService`] serves.
    pub(crate) fn routes(self) -> Router {
        Router::new()
            .route(routes::emails::LIST, post(Self::add))
            .route(routes::emails::VERIFY, post(Self::verify))
            .with_state(self)
    }

    /// Adds an unverified address to the owner's account, and queues its
    /// verification email.
    async fn add(
        State(app): State<Self>,
        owner: ConfirmedOwner,
        Json(body): Json<AddEmail>,
    ) -> Result<(StatusCode, Json<EmailView>), ApiError> {
        let added = AccountService::new(app.database.clone())
            .add_email(&owner, &app.config, &body.email)
            .await?;
        Ok((StatusCode::CREATED, Json(added)))
    }

    /// Verifies the address whose link holds the body's token, whether this
    /// browser is signed out or signed in to the account the address joins.
    async fn verify(
        State(app): State<Self>,
        context: RequestContext,
        owner: Option<Owner>,
        Json(body): Json<VerifyEmail>,
    ) -> Result<Json<Verified>, ApiError> {
        let verified = AccountService::new(app.database.clone())
            .verify_email(&body.token, owner.as_ref(), context)
            .await?;
        Ok(Json(verified))
    }

    /// Checks that the sender can send to `address`, which is stricter than
    /// [`NormalizedEmail::parse`]: a link mailed to an address it can't send to
    /// would never arrive.
    pub(crate) fn sendable(address: &NormalizedEmail) -> Result<(), lettre::address::AddressError> {
        address.as_str().parse::<lettre::Address>().map(drop)
    }
}

/// Account `user_id`'s addresses in `tx`, changed as `context`'s. Every
/// address joins, is verified and leaves through it, so each is recorded and
/// counted the same way.
pub(crate) struct Emails<'a, 'tx> {
    /// The transaction that makes the changes.
    tx: &'a mut DatabaseTransaction<'tx>,
    /// Who makes them.
    context: &'a EventContext,
    /// The account whose addresses they are.
    user_id: UserId,
}

impl<'a, 'tx> Emails<'a, 'tx> {
    /// Changes `user_id`'s addresses in `tx`, as `context`'s.
    pub(crate) const fn new(
        tx: &'a mut DatabaseTransaction<'tx>,
        context: &'a EventContext,
        user_id: UserId,
    ) -> Self {
        Self {
            tx,
            context,
            user_id,
        }
    }

    /// Records the removal of address `id`, then deletes it. In that order, so
    /// the `email` channel still reads a verified address and copies it into the
    /// notice of its removal.
    pub(crate) async fn delete(&mut self, id: EmailId) -> Result<(), DatabaseError> {
        let removed = DomainEvent::EmailRemoved { email_id: id };
        self.tx
            .record(self.context, Some(self.user_id), removed)
            .await?;
        self.tx.emails().remove(self.user_id, id).await
    }

    /// [`Self::delete`], without the notice of its removal: for a step of an
    /// operation whose own notification tells the account, such as the
    /// unverified addresses support's recovery removes.
    pub(crate) async fn delete_quietly(&mut self, id: EmailId) -> Result<(), DatabaseError> {
        let removed = DomainEvent::EmailRemoved { email_id: id };
        self.tx
            .record_quietly(self.context, Some(self.user_id), removed)
            .await?;
        self.tx.emails().remove(self.user_id, id).await
    }

    /// Counts one verification email to `address` against the hourly limits of
    /// the account, `client_ip` and the address's mailbox, so neither one
    /// account, nor one client, nor many of both can flood a mailbox. In the
    /// caller's transaction, so a rejected or failed addition counts nothing.
    async fn limit(
        &mut self,
        client_ip: IpAddr,
        address: &NormalizedEmail,
    ) -> Result<(), EmailError> {
        let buckets = [
            (
                format!("account:{}", self.user_id),
                VERIFICATIONS_PER_ACCOUNT,
            ),
            (format!("ip:{client_ip}"), VERIFICATIONS_PER_IP),
            (
                format!("address:{}", address.mailbox()),
                VERIFICATIONS_PER_ADDRESS,
            ),
        ];
        for (key, limit) in buckets {
            let bucket = Digest::sha256(format!("verification-email:{key}").as_bytes());
            if !self
                .tx
                .security()
                .allow_request(bucket.as_bytes(), limit, VERIFICATION_WINDOW)
                .await?
            {
                return Err(EmailError::RateLimited { limit });
            }
        }
        Ok(())
    }

    /// Records `EmailVerified` for address `email_id`, then `PrimaryEmailChanged`
    /// with no previous address if it became the `primary` one, the account
    /// having had none. Shared by `verify_email` and [`Self::add_verified`].
    async fn record_verified(
        &mut self,
        email_id: EmailId,
        primary: bool,
    ) -> Result<(), DatabaseError> {
        let subject = Some(self.user_id);
        self.tx
            .record(
                self.context,
                subject,
                DomainEvent::EmailVerified { email_id },
            )
            .await?;
        if primary {
            let event = DomainEvent::PrimaryEmailChanged {
                email_id,
                previous_email_id: None,
            };
            self.tx.record(self.context, subject, event).await?;
        }
        Ok(())
    }

    /// Refuses another address for an account that holds `count`.
    const fn room(count: usize) -> Result<(), EmailError> {
        if count >= MAX_EMAILS {
            return Err(EmailError::TooMany { max: MAX_EMAILS });
        }
        Ok(())
    }

    /// Imports the address `pending`'s provider verified, from `client_ip`,
    /// while linking that identity. It joins verified and primary when the
    /// provider is authoritative for it; otherwise unverified, with its
    /// verification email, as when the owner adds it, since it may have changed
    /// owners since the provider verified it.
    /// Only an account with no verified address and without it takes one, and
    /// never one another account holds verified, even while that one is disabled
    /// or being deleted: a provider never replaces the owner's own address or takes
    /// someone else's. An address that would be one
    /// too many, or whose link cannot be sent now, is skipped, since signing in
    /// matters more: the owner can add it later. Another account verifying the
    /// same address in the same instant still fails the ownership trigger, and so
    /// the transaction; retrying skips the address.
    pub(crate) async fn import(
        &mut self,
        config: &EmailConfig,
        client_ip: IpAddr,
        pending: &PendingFederation,
    ) -> Result<(), EmailError> {
        let Some(address) = &pending.email else {
            return Ok(());
        };
        let address = NormalizedEmail::parse(address).map_err(EmailError::Invalid)?;
        let emails = self.tx.emails().list(self.user_id).await?;
        if emails
            .iter()
            .any(|email| email.verified_at.is_some() || email.normalized_email == address.as_str())
            || self
                .tx
                .emails()
                .is_owned(address.as_str(), Some(self.user_id))
                .await?
        {
            return Ok(());
        }
        let provider = Some(pending.provider);
        let imported = match Self::room(emails.len()) {
            Ok(()) if pending.email_authoritative => self.add_verified(&address, provider).await,
            Ok(()) => self
                .add_unverified(&address, provider, config, client_ip)
                .await
                .map(drop),
            Err(error) => Err(error),
        };
        match imported {
            Err(
                error @ (EmailError::TooMany { .. }
                | EmailError::Disabled
                | EmailError::Unsendable(_)
                | EmailError::RateLimited { .. }),
            ) => {
                tracing::info!(error = %ErrorChain(&error), provider = pending.provider.as_str(), "provider address not imported");
                Ok(())
            }
            imported => imported,
        }
    }

    /// Starts the new account with `address`, the one the invitation it
    /// redeemed was mailed to, verified and primary: opening the mailed link
    /// proved what a verification email would. An address another account
    /// verified since the invitation was mailed is skipped, as a provider's is,
    /// and the account starts without one.
    pub(crate) async fn invited(&mut self, address: &NormalizedEmail) -> Result<(), EmailError> {
        if self
            .tx
            .emails()
            .is_owned(address.as_str(), Some(self.user_id))
            .await?
        {
            tracing::info!("invited address held by another account; not added");
            return Ok(());
        }
        self.add_verified(address, None).await
    }

    /// Adds `address` unverified, imported from `provider` if any, and records
    /// it with its verification email, counted against the hourly limits of the
    /// account, `client_ip` and the mailbox. Returns its ID.
    async fn add_unverified(
        &mut self,
        address: &NormalizedEmail,
        provider: Option<Provider>,
        config: &EmailConfig,
        client_ip: IpAddr,
    ) -> Result<EmailId, EmailError> {
        if config.smtp.is_none() {
            return Err(EmailError::Disabled);
        }
        EmailApplication::sendable(address).map_err(EmailError::Unsendable)?;
        self.limit(client_ip, address).await?;
        let id = EmailId::new();
        let digest = config.verification_keys.token(id).digest();
        self.tx
            .emails()
            .insert_unverified(&InsertUnverifiedEmail {
                id,
                user_id: self.user_id,
                normalized_email: address.as_str(),
                secret_digest: digest.as_bytes(),
                expires_at: Utc::now() + VERIFICATION_LIFETIME,
            })
            .await?;
        self.record_added(id, address, provider).await?;
        Ok(id)
    }

    /// Adds `address` verified, imported from `provider` if any, as the primary
    /// one of an account that has none, which a provider authoritative for it,
    /// or the invitation mailed to it, vouches for.
    async fn add_verified(
        &mut self,
        address: &NormalizedEmail,
        provider: Option<Provider>,
    ) -> Result<(), EmailError> {
        let id = EmailId::new();
        self.tx
            .emails()
            .insert_verified(&InsertVerifiedEmail {
                id,
                user_id: self.user_id,
                normalized_email: address.as_str().to_owned(),
                is_primary: true,
                verified_at: Utc::now(),
            })
            .await?;
        self.record_added(id, address, provider).await?;
        self.record_verified(id, true).await?;
        Ok(())
    }

    /// Records the addition of `address`, imported from `provider` if any, as `id`.
    async fn record_added(
        &mut self,
        id: EmailId,
        address: &NormalizedEmail,
        provider: Option<Provider>,
    ) -> Result<(), EmailError> {
        let added = DomainEvent::EmailAdded {
            email_id: id,
            address: Personal::Value(address.clone()),
            provider,
        };
        self.tx
            .record(self.context, Some(self.user_id), added)
            .await?;
        Ok(())
    }
}

/// The account's addresses. Any administrator may also list them, choose the
/// primary one among verified ones, and remove one; each change queues the same
/// notices, whoever makes it. Only the owner adds an address, since it receives
/// the account's mail, and verifying one is authorized by its link's token.
impl AccountService {
    /// Lists the account's addresses, primary first.
    pub(crate) async fn emails(&self, viewer: &Viewer) -> Result<Vec<EmailView>, AccountError> {
        let read = viewer.authorize(&self.database).await?;
        self.authorized_emails(&read).await
    }

    /// [`Self::emails`], for a read already authorized.
    pub(crate) async fn authorized_emails(
        &self,
        read: &Authorized,
    ) -> Result<Vec<EmailView>, AccountError> {
        let emails = self.database.emails().list(read.user()).await?;
        Ok(emails.into_iter().map(EmailView::from).collect())
    }

    /// Adds an unverified address to the owner's account and queues its
    /// verification email, in one transaction, once they confirmed with a passkey.
    /// Adding an address that is still waiting for verification replaces it,
    /// which sends a new link. The address may be another account's: rejecting it
    /// here would tell anyone which addresses have accounts, so only verification,
    /// by whoever controls the mailbox, finds out. Only the owner adds an address,
    /// since it receives the account's mail.
    pub(crate) async fn add_email(
        &self,
        owner: &Owner,
        config: &EmailConfig,
        address: &str,
    ) -> Result<EmailView, AccountError> {
        let address = NormalizedEmail::parse(address).map_err(EmailError::Invalid)?;
        let user = owner.user();
        let context = owner.context();
        let mut tx = self.database.begin().await?;
        owner.lock_confirmed(&mut tx).await?;
        let emails = tx.emails().list(user).await?;
        let mut count = emails.len();
        let mut changes = Emails::new(&mut tx, &context, user);
        if let Some(existing) = emails
            .iter()
            .find(|email| email.normalized_email == address.as_str())
        {
            if existing.verified_at.is_some() {
                return Err(EmailError::AlreadyAdded.into());
            }
            changes.delete(existing.id).await?;
            count -= 1;
        }
        Emails::room(count)?;
        let id = changes
            .add_unverified(&address, None, config, owner.request().client_ip)
            .await?;
        tx.commit().await?;
        Ok(EmailView {
            id,
            address: address.into(),
            verified: false,
            primary: false,
        })
    }

    /// Verifies the address whose link holds `token`, once. It becomes primary if
    /// the account had none. The token authorizes it, not a session: it proves
    /// control of the mailbox, and whoever holds it may be signed out or on another
    /// device. A browser `signed_in` to another account is refused, since it would
    /// confirm the address for an account that is not theirs, as a phishing link
    /// could ask. Pages never verify on `GET`, since mail scanners follow links.
    pub(crate) async fn verify_email(
        &self,
        token: &str,
        signed_in: Option<&Owner>,
        request: RequestContext,
    ) -> Result<Verified, AccountError> {
        // A token that does not parse cannot match: answer as for an unknown one.
        let digest = Secret::parse(token)
            .map_err(|_| EmailError::InvalidToken)?
            .digest();
        let mut tx = self.database.begin().await?;
        let pending = tx
            .emails()
            .find_verification(digest.as_bytes())
            .await?
            .ok_or(EmailError::InvalidToken)?;
        tx.users()
            .find_active_by_id_for_update(pending.user_id)
            .await?
            .ok_or(EmailError::InvalidToken)?;
        let context = match signed_in {
            Some(owner) if owner.user() == pending.user_id => owner.context(),
            Some(_) => return Err(EmailError::OtherAccount.into()),
            None => request.actor(Actor::Anonymous),
        };
        let primary = match tx
            .emails()
            .consume_verification(&pending, digest.as_bytes())
            .await
        {
            Ok(primary) => primary,
            Err(DatabaseError::NotFound) => return Err(EmailError::InvalidToken.into()),
            // The ownership trigger: another account, of any status, verified it first.
            Err(DatabaseError::Conflict(_)) => return Err(EmailError::OwnedElsewhere.into()),
            Err(error) => return Err(error.into()),
        };
        Emails::new(&mut tx, &context, pending.user_id)
            .record_verified(pending.email_id, primary)
            .await?;
        tx.commit().await?;
        Ok(Verified {
            address: pending.address,
            primary,
        })
    }

    /// Makes a verified address the primary one, and queues a notice to the
    /// previous primary address.
    pub(crate) async fn make_primary_email(
        &self,
        subject: &Subject,
        id: EmailId,
    ) -> Result<(), AccountError> {
        let mut tx = self.database.begin().await?;
        subject.lock_confirmed(&mut tx).await?;
        let email = tx
            .emails()
            .find(subject.user(), id)
            .await?
            .ok_or(EmailError::NotFound)?;
        if email.verified_at.is_none() {
            return Err(EmailError::NotVerified.into());
        }
        if !email.is_primary {
            let previous_email_id = tx.emails().set_primary(subject.user(), id).await?;
            tx.record(
                &subject.context(),
                Some(subject.user()),
                DomainEvent::PrimaryEmailChanged {
                    email_id: id,
                    previous_email_id,
                },
            )
            .await?;
        }
        subject.commit(tx).await
    }

    /// Removes an address, and queues a notice to it if it was verified. The
    /// primary address can only be removed once no other verified address could
    /// replace it: another one must be made primary first, which notifies it too.
    pub(crate) async fn remove_email(
        &self,
        subject: &Subject,
        id: EmailId,
    ) -> Result<(), AccountError> {
        let mut tx = self.database.begin().await?;
        subject.lock_confirmed(&mut tx).await?;
        let emails = tx.emails().list(subject.user()).await?;
        let email = emails
            .iter()
            .find(|email| email.id == id)
            .ok_or(EmailError::NotFound)?;
        if email.is_primary
            && emails
                .iter()
                .any(|other| other.id != id && other.verified_at.is_some())
        {
            return Err(EmailError::PrimaryInUse.into());
        }
        Emails::new(&mut tx, &subject.context(), subject.user())
            .delete(id)
            .await?;
        subject.commit(tx).await
    }
}

/// Supplies the shared database boundary to browser-session extractors.
impl axum::extract::FromRef<EmailApplication> for Database {
    /// Shares persistence with the session extractors.
    fn from_ref(application: &EmailApplication) -> Self {
        application.database.clone()
    }
}

/// Database-backed address scenarios.
#[cfg(test)]
mod tests;

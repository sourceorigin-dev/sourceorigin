//! The resources an account owns, and the [`AccountService`] operations on them,
//! for the owner only. Every operation finds the resource as the owner's first,
//! so any other resource is not found, and nothing checks ownership again.

use super::{
    InvalidResourceInput, NewResource, ResourceEdit, ResourceRegistry, ResourceRegistryError,
};
use crate::{
    account::{AccountError, AccountService, Owner},
    security::ApiError,
};
use sourceorigin_common::{
    ResourceId,
    admin::{Resource, ResourceDetail},
    clients::ClientOwner,
};
use thiserror::Error;

/// One of an owner's resources, with its owner. Only
/// [`AccountService::owned_resource`] builds one, once it checked the resource is
/// the owner's, so the operations on it never check ownership again.
pub(crate) struct OwnedResource {
    /// The owner.
    owner: Owner,
    /// The resource, as it was when found.
    resource: ResourceDetail,
}

impl OwnedResource {
    /// The resource, its scopes and the clients it admits, as they were when
    /// found.
    pub(crate) const fn detail(&self) -> &ResourceDetail {
        &self.resource
    }
}

/// Why an owner's request about their resources was refused.
#[derive(Debug, Error)]
pub(crate) enum OwnedResourceError {
    /// The account owns no resource with this ID: another account's included, so
    /// probing IDs reveals nothing.
    #[error("resource not found")]
    NotFound,
    /// A form's field holds something else than it asks for.
    #[error(transparent)]
    Invalid(#[from] InvalidResourceInput),
    /// The registry refused the change, or failed.
    #[error(transparent)]
    Registry(#[from] ResourceRegistryError),
}

/// The resources only their owner registers and changes. Administrators manage
/// every resource through `AdminService` instead.
impl AccountService {
    /// Lists the resources the owner manages, by identifier.
    pub(crate) async fn resources(&self, owner: &Owner) -> Result<Vec<Resource>, AccountError> {
        Ok(self.database.resources().list(Some(owner.user())).await?)
    }

    /// The owner's resource `id`, as it is now. Any other ID, another account's
    /// included, is not found. Every operation on one resource starts here.
    pub(crate) async fn owned_resource(
        &self,
        owner: Owner,
        id: &str,
    ) -> Result<OwnedResource, AccountError> {
        let found = match id.parse::<ResourceId>() {
            Ok(id) => self.database.resources().detail(id).await?,
            Err(_) => None,
        };
        let resource = found
            .filter(|detail| detail.resource.owner_id == Some(owner.user()))
            .ok_or(OwnedResourceError::NotFound)?;
        Ok(OwnedResource { owner, resource })
    }

    /// Runs `change` on the resource registry in one transaction, attributed to
    /// `owner`, once they confirmed with a passkey under the account lock, and
    /// commits it. The registry records each change's event itself.
    async fn change_resources<T>(
        &self,
        owner: &Owner,
        change: impl AsyncFnOnce(&mut ResourceRegistry<'_, '_>) -> Result<T, ResourceRegistryError>,
    ) -> Result<T, AccountError> {
        let mut tx = self.database.begin().await?;
        owner.lock_confirmed(&mut tx).await?;
        let value = change(&mut ResourceRegistry::new(&mut tx, owner.context()))
            .await
            .map_err(OwnedResourceError::from)?;
        tx.commit().await?;
        Ok(value)
    }

    /// Registers the resource `new`, which the owner manages, once they
    /// confirmed with a passkey, and returns its ID.
    pub(crate) async fn create_resource(
        &self,
        owner: &Owner,
        new: &NewResource,
    ) -> Result<ResourceId, AccountError> {
        self.change_resources(owner, async |resources| {
            let owner = ClientOwner::Account(owner.user());
            resources.create(owner, &new.identifier, &new.name).await
        })
        .await
    }

    /// Makes `edit` to an owner's resource, once they confirmed with a passkey:
    /// with a stolen session cookie, someone could otherwise let a client they
    /// control ask for the resource's tokens.
    pub(crate) async fn edit_resource(
        &self,
        owned: &OwnedResource,
        edit: ResourceEdit,
    ) -> Result<(), AccountError> {
        let id = owned.resource.resource.id;
        self.change_resources(&owned.owner, async |resources| {
            resources.edit(id, edit).await
        })
        .await
    }
}

impl From<OwnedResourceError> for ApiError {
    /// Answers as each failure's own mapping does.
    fn from(error: OwnedResourceError) -> Self {
        match error {
            OwnedResourceError::NotFound => Self::not_found("You have no resource with this ID."),
            OwnedResourceError::Invalid(error) => error.into(),
            OwnedResourceError::Registry(error) => error.into(),
        }
    }
}

impl From<InvalidResourceInput> for ApiError {
    /// A field that holds something else says what it takes.
    fn from(error: InvalidResourceInput) -> Self {
        Self::bad_request(error.to_string())
    }
}

impl From<ResourceRegistryError> for ApiError {
    /// A refusal says what to fix, in the registry's own words, to an owner and
    /// an administrator alike; a failure of the database is internal.
    fn from(error: ResourceRegistryError) -> Self {
        let message = error.to_string();
        match error {
            ResourceRegistryError::Unknown | ResourceRegistryError::NoAccess(_) => {
                Self::not_found(message)
            }
            ResourceRegistryError::Exists { .. }
            | ResourceRegistryError::QuotaExceeded
            | ResourceRegistryError::TooManyScopes => Self::conflict(message),
            ResourceRegistryError::ReservedName(_)
            | ResourceRegistryError::UnknownScope(_)
            | ResourceRegistryError::UnknownClient(_) => Self::bad_request(message),
            ResourceRegistryError::Database(error) => error.into(),
        }
    }
}

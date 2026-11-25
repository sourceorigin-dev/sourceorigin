//! The one place that changes resources: the APIs access tokens are issued for
//! (RFC 8707), the scopes each defines, and the clients each admits. Each change
//! happens in the caller's transaction, with the event that records it, so the
//! caller can check inside the same transaction that its user still may make it.
//! `AdminService` uses it for the `resource` command and the dashboard, and
//! `AccountService` for the resources accounts own ([`owned`]).
//!
//! Nothing is cached: every request reads the access it needs from the database,
//! so a change applies to the next request on every replica.

use sourceorigin_common::{
    ClientId, InvalidClientId, ResourceId, UserId,
    admin::Resource,
    clients::{
        ClientOwner, DisplayName, InvalidDisplayName, InvalidScopeDescription, ReservedDisplayName,
        ScopeDescription,
    },
    events::{DomainEvent, EventContext},
    protocol::resource::{
        InvalidResourceIdentifier, InvalidResourceScope, ResourceIdentifier, ResourceScope,
    },
};
use sourceorigin_repository::{DatabaseError, DatabaseTransaction};
use thiserror::Error;

/// The resources an account owns: the owner's operations.
mod owned;
pub(crate) use owned::{OwnedResource, OwnedResourceError};

/// Resources an account may manage at once. Deleting one frees its place.
const MAX_OWNED_RESOURCES: u32 = 10;
/// Scopes a resource may define.
const MAX_RESOURCE_SCOPES: usize = 32;

/// Registers, renames and deletes resources, defines their scopes and grants
/// clients access to them, in one transaction, attributing each change to one
/// actor.
pub(crate) struct ResourceRegistry<'a, 't> {
    /// Where the changes and their events are written; the caller commits it.
    tx: &'a mut DatabaseTransaction<'t>,
    /// Who makes the changes, and where their request came from.
    context: EventContext,
}

/// A change to a resource that exists: every one the console, the dashboard and
/// the `resource` command make, so all three go through [`ResourceRegistry::edit`].
#[derive(Clone, Debug)]
pub(crate) enum ResourceEdit {
    /// Give it another name.
    Rename(DisplayName),
    /// Define a scope, or describe it anew.
    PutScope(ResourceScope, ScopeDescription),
    /// Remove a scope, by name.
    RemoveScope(String),
    /// Let a client ask for its tokens, with these scopes of it.
    Grant(ClientId, Vec<ResourceScope>),
    /// End a client's access to it.
    Revoke(ClientId),
    /// Delete it for good.
    Delete,
}

/// What a form's field holds that is not what it asks for. Each says what the
/// field takes, for the form to show.
#[derive(Debug, Error)]
pub(crate) enum InvalidResourceInput {
    /// The identifier is not an absolute https URI.
    #[error(transparent)]
    Identifier(#[from] InvalidResourceIdentifier),
    /// The name is blank or too long.
    #[error(transparent)]
    Name(#[from] InvalidDisplayName),
    /// A scope is not one a resource may define.
    #[error(transparent)]
    Scope(#[from] InvalidResourceScope),
    /// A scope's description is blank or too long.
    #[error(transparent)]
    Description(#[from] InvalidScopeDescription),
    /// The client ID is not one.
    #[error(transparent)]
    Client(#[from] InvalidClientId),
}

/// A resource to register: its identifier and the name consent screens show.
pub(crate) struct NewResource {
    /// The identifier, which its tokens carry as their audience.
    pub(crate) identifier: ResourceIdentifier,
    /// The name.
    pub(crate) name: DisplayName,
}

impl NewResource {
    /// The resource a form typed, its fields trimmed.
    ///
    /// # Errors
    /// Fails if the identifier or the name is not one.
    pub(crate) fn typed(identifier: &str, name: &str) -> Result<Self, InvalidResourceInput> {
        Ok(Self {
            identifier: identifier.trim().parse()?,
            name: name.trim().parse()?,
        })
    }
}

/// The edits a form typed, its fields trimmed: the console's and the dashboard's
/// forms read theirs here.
impl ResourceEdit {
    /// Renaming to `name`.
    ///
    /// # Errors
    /// Fails if the name is blank or too long.
    pub(crate) fn rename(name: &str) -> Result<Self, InvalidResourceInput> {
        Ok(Self::Rename(name.trim().parse()?))
    }

    /// Defining the scope `name`, which `description` describes.
    ///
    /// # Errors
    /// Fails if either is not one.
    pub(crate) fn put_scope(name: &str, description: &str) -> Result<Self, InvalidResourceInput> {
        Ok(Self::PutScope(
            name.trim().parse()?,
            description.trim().parse()?,
        ))
    }

    /// Granting the client `client` the scopes `scopes` lists, separated by
    /// spaces.
    ///
    /// # Errors
    /// Fails if the client ID or a scope is not one.
    pub(crate) fn grant(client: &str, scopes: &str) -> Result<Self, InvalidResourceInput> {
        let scopes = scopes
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        Ok(Self::Grant(client.trim().parse()?, scopes))
    }

    /// Ending the access of the client `client`.
    ///
    /// # Errors
    /// Fails if the client ID is not one.
    pub(crate) fn revoke(client: &str) -> Result<Self, InvalidResourceInput> {
        Ok(Self::Revoke(client.trim().parse()?))
    }
}

/// Why a change to a resource was refused.
#[derive(Debug, Error)]
pub(crate) enum ResourceRegistryError {
    /// Another resource has this identifier.
    #[error("resource {identifier} is already registered")]
    Exists {
        /// The requested identifier.
        identifier: ResourceIdentifier,
        /// The uniqueness violation.
        #[source]
        source: DatabaseError,
    },
    /// No such resource exists.
    #[error("no such resource is registered")]
    Unknown,
    /// An account's resource may not be named after a reserved brand.
    #[error(transparent)]
    ReservedName(#[from] ReservedDisplayName),
    /// The account already manages [`MAX_OWNED_RESOURCES`] resources.
    #[error("an account may manage at most {MAX_OWNED_RESOURCES} resources")]
    QuotaExceeded,
    /// The resource already defines [`MAX_RESOURCE_SCOPES`] scopes.
    #[error("a resource may define at most {MAX_RESOURCE_SCOPES} scopes")]
    TooManyScopes,
    /// The resource does not define this scope.
    #[error("the resource defines no scope {0}")]
    UnknownScope(String),
    /// No client that could be granted access has this ID.
    #[error("no client has ID {0}")]
    UnknownClient(ClientId),
    /// The client has no access to the resource.
    #[error("client {0} has no access to the resource")]
    NoAccess(ClientId),
    /// The database failed.
    #[error("resource database operation failed")]
    Database(#[from] DatabaseError),
}

impl<'a, 't> ResourceRegistry<'a, 't> {
    /// Changes resources in `tx`, recording each change as made in `context`.
    #[must_use]
    pub(crate) const fn new(tx: &'a mut DatabaseTransaction<'t>, context: EventContext) -> Self {
        Self { tx, context }
    }

    /// Registers the resource `identifier`, named `name`, managed by `owner`, and
    /// returns its ID. An account may manage at most [`MAX_OWNED_RESOURCES`], and
    /// may not name one after a reserved brand, which would read as SourceOrigin's
    /// own on the consent page.
    ///
    /// # Errors
    /// Fails if the identifier is taken, the name is reserved for an account, the
    /// owner's quota is used up, or the database fails.
    pub(crate) async fn create(
        &mut self,
        owner: ClientOwner,
        identifier: &ResourceIdentifier,
        name: &DisplayName,
    ) -> Result<ResourceId, ResourceRegistryError> {
        if let ClientOwner::Account(user_id) = owner {
            name.ensure_unreserved()?;
            self.reserve(user_id).await?;
        }
        let id = ResourceId::new();
        self.tx
            .resources()
            .create(id, identifier, name, owner)
            .await
            .map_err(|error| match error {
                DatabaseError::Conflict(_) => ResourceRegistryError::Exists {
                    identifier: identifier.clone(),
                    source: error,
                },
                error => error.into(),
            })?;
        let registered = DomainEvent::ResourceRegistered { resource_id: id };
        self.tx.record(&self.context, None, registered).await?;
        Ok(id)
    }

    /// Checks that `owner` may register one more resource. Their account stays
    /// locked until the transaction ends, so concurrent registrations count one
    /// another.
    async fn reserve(&mut self, owner: UserId) -> Result<(), ResourceRegistryError> {
        self.tx.users().find_by_id_for_update(owner).await?;
        if self.tx.resources().count_owned(owner).await? >= u64::from(MAX_OWNED_RESOURCES) {
            return Err(ResourceRegistryError::QuotaExceeded);
        }
        Ok(())
    }

    /// Makes `edit` to the resource `id`, and records the event the edit
    /// answers, about no account, as made in this registry's context. An edit
    /// that changes nothing answers none.
    ///
    /// # Errors
    /// Fails as the method making the edit does.
    pub(crate) async fn edit(
        &mut self,
        id: ResourceId,
        edit: ResourceEdit,
    ) -> Result<(), ResourceRegistryError> {
        let event = match edit {
            ResourceEdit::Rename(name) => self.rename(id, &name).await?,
            ResourceEdit::PutScope(name, description) => {
                Some(self.put_scope(id, &name, &description).await?)
            }
            ResourceEdit::RemoveScope(name) => Some(self.remove_scope(id, &name).await?),
            ResourceEdit::Grant(client, scopes) => Some(self.grant(&client, id, &scopes).await?),
            ResourceEdit::Revoke(client) => Some(self.revoke(&client, id).await?),
            ResourceEdit::Delete => Some(self.delete(id).await?),
        };
        if let Some(event) = event {
            self.tx.record(&self.context, None, event).await?;
        }
        Ok(())
    }

    /// Renames the resource `id`, and answers the event to record: none for a
    /// name it already has.
    ///
    /// # Errors
    /// Fails if the resource does not exist, the name is reserved for the account
    /// that owns it, or the database fails.
    async fn rename(
        &mut self,
        id: ResourceId,
        name: &DisplayName,
    ) -> Result<Option<DomainEvent>, ResourceRegistryError> {
        let resource = self.locked(id).await?;
        if resource.owner_id.is_some() {
            name.ensure_unreserved()?;
        }
        if resource.display_name == name.as_str() {
            return Ok(None);
        }
        self.tx.resources().rename(id, name).await?;
        Ok(Some(DomainEvent::ResourceUpdated { resource_id: id }))
    }

    /// Deletes the resource `id` for good: no client gets tokens for it any
    /// more, and the refresh grants and consents for it end.
    ///
    /// # Errors
    /// Fails if the resource does not exist or the database fails.
    async fn delete(&mut self, id: ResourceId) -> Result<DomainEvent, ResourceRegistryError> {
        if !self.tx.resources().delete(id).await? {
            return Err(ResourceRegistryError::Unknown);
        }
        Ok(DomainEvent::ResourceDeleted { resource_id: id })
    }

    /// Defines the scope `name` of the resource `id`, described as `description`,
    /// or describes it anew.
    ///
    /// # Errors
    /// Fails if the resource does not exist, already defines
    /// [`MAX_RESOURCE_SCOPES`] others, or the database fails.
    async fn put_scope(
        &mut self,
        id: ResourceId,
        name: &ResourceScope,
        description: &ScopeDescription,
    ) -> Result<DomainEvent, ResourceRegistryError> {
        self.locked(id).await?;
        let defined = self.tx.resources().scopes(id).await?;
        let known = defined.iter().any(|scope| scope.name == name.as_str());
        if !known && defined.len() >= MAX_RESOURCE_SCOPES {
            return Err(ResourceRegistryError::TooManyScopes);
        }
        self.tx.resources().put_scope(id, name, description).await?;
        Ok(DomainEvent::ResourceUpdated { resource_id: id })
    }

    /// Removes the scope `name` from the resource `id`, and from what every
    /// client may request of it. Refresh grants that hold it are refused at their
    /// next use.
    ///
    /// # Errors
    /// Fails if the resource does not exist or defines no such scope, or the
    /// database fails.
    async fn remove_scope(
        &mut self,
        id: ResourceId,
        name: &str,
    ) -> Result<DomainEvent, ResourceRegistryError> {
        self.locked(id).await?;
        if !self.tx.resources().remove_scope(id, name).await? {
            return Err(ResourceRegistryError::UnknownScope(name.to_owned()));
        }
        Ok(DomainEvent::ResourceUpdated { resource_id: id })
    }

    /// Lets `client` ask for tokens for the resource `id` with `scopes`, each of
    /// which the resource must define. They replace any the client could request
    /// before: a refresh grant holding one it loses is refused at its next use.
    ///
    /// # Errors
    /// Fails if the resource or the client does not exist, the client was
    /// deleted, the resource does not define a scope, or the database fails.
    async fn grant(
        &mut self,
        client: &ClientId,
        id: ResourceId,
        scopes: &[ResourceScope],
    ) -> Result<DomainEvent, ResourceRegistryError> {
        self.locked(id).await?;
        let defined = self.tx.resources().scopes(id).await?;
        if let Some(unknown) = scopes
            .iter()
            .find(|scope| !defined.iter().any(|known| known.name == scope.as_str()))
        {
            return Err(ResourceRegistryError::UnknownScope(unknown.to_string()));
        }
        let registered = self.tx.oidc().find_client(client.as_str()).await?;
        if registered.is_none_or(|client| client.deleted_at.is_some()) {
            return Err(ResourceRegistryError::UnknownClient(client.clone()));
        }
        self.tx.resources().grant_access(client, id, scopes).await?;
        Ok(DomainEvent::ResourceAccessGranted {
            resource_id: id,
            client_id: client.clone(),
            scopes: scopes.iter().map(ResourceScope::to_string).collect(),
        })
    }

    /// Ends `client`'s access to the resource `id`: it gets no more tokens for
    /// it, and its refresh grants for it are revoked.
    ///
    /// # Errors
    /// Fails if the resource does not exist, the client had no access to it, or
    /// the database fails.
    async fn revoke(
        &mut self,
        client: &ClientId,
        id: ResourceId,
    ) -> Result<DomainEvent, ResourceRegistryError> {
        self.locked(id).await?;
        if !self.tx.resources().revoke_access(client, id).await? {
            return Err(ResourceRegistryError::NoAccess(client.clone()));
        }
        Ok(DomainEvent::ResourceAccessRevoked {
            resource_id: id,
            client_id: client.clone(),
        })
    }

    /// The resource `id`, locked until the transaction ends, so changes to it
    /// take turns.
    async fn locked(&mut self, id: ResourceId) -> Result<Resource, ResourceRegistryError> {
        self.tx
            .resources()
            .lock(id)
            .await?
            .ok_or(ResourceRegistryError::Unknown)
    }
}

/// Resources against a real database, through their registry.
#[cfg(test)]
mod tests;

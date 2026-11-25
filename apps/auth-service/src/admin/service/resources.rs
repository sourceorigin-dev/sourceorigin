//! The APIs registered as resources, the scopes they define and the clients
//! they admit: every one, whoever manages it.

use super::AdminService;
use crate::{
    admin::{AdminError, Operator, Reason},
    resources::{ResourceEdit, ResourceRegistry, ResourceRegistryError},
};
use sourceorigin_common::{
    ResourceId,
    admin::{Resource, ResourceDetail},
    clients::{ClientOwner, DisplayName},
    protocol::resource::ResourceIdentifier,
};

impl AdminService {
    /// Lists every resource, by identifier.
    pub(crate) async fn list_resources(
        &self,
        operator: &Operator,
    ) -> Result<Vec<Resource>, AdminError> {
        operator.authorize_read(&self.database).await?;
        Ok(self.database.resources().list(None).await?)
    }

    /// Describes the resource `id`, with its scopes and the clients it admits.
    pub(crate) async fn resource_detail(
        &self,
        operator: &Operator,
        id: ResourceId,
    ) -> Result<ResourceDetail, AdminError> {
        operator.authorize_read(&self.database).await?;
        let detail = self.database.resources().detail(id).await?;
        Ok(detail.ok_or(ResourceRegistryError::Unknown)?)
    }

    /// The resource whose identifier is `identifier`, which the `resource`
    /// command names resources by.
    pub(crate) async fn find_resource(
        &self,
        operator: &Operator,
        identifier: &ResourceIdentifier,
    ) -> Result<Resource, AdminError> {
        operator.authorize_read(&self.database).await?;
        let found = self
            .database
            .resources()
            .find_by_identifier(identifier.as_str())
            .await?;
        Ok(found.ok_or(ResourceRegistryError::Unknown)?)
    }

    /// Runs `change` on the resource registry in one transaction, attributed to
    /// `operator` with their `reason`, and commits once the operator is
    /// authorized again. The registry records each change's event itself.
    async fn change_resources<T>(
        &self,
        operator: &Operator,
        reason: Reason,
        change: impl AsyncFnOnce(&mut ResourceRegistry<'_, '_>) -> Result<T, ResourceRegistryError>,
    ) -> Result<T, AdminError> {
        let mut tx = self.database.begin().await?;
        let context = operator.context(&reason);
        let value = change(&mut ResourceRegistry::new(&mut tx, context)).await?;
        Self::finish(tx, operator, &reason, None, Vec::new()).await?;
        Ok(value)
    }

    /// Registers the resource `identifier`, named `name`, which operators
    /// manage, and returns its ID.
    pub(crate) async fn create_resource(
        &self,
        operator: &Operator,
        identifier: &ResourceIdentifier,
        name: &DisplayName,
        reason: Reason,
    ) -> Result<ResourceId, AdminError> {
        self.change_resources(operator, reason, async |resources| {
            resources
                .create(ClientOwner::Operator, identifier, name)
                .await
        })
        .await
    }

    /// Makes `edit` to the resource `id`: its name, a scope it defines, or a
    /// client's access to it.
    pub(crate) async fn edit_resource(
        &self,
        operator: &Operator,
        id: ResourceId,
        edit: ResourceEdit,
        reason: Reason,
    ) -> Result<(), AdminError> {
        self.change_resources(operator, reason, async |resources| {
            resources.edit(id, edit).await
        })
        .await
    }

    /// Deletes the resource `id` for good: no client gets tokens for it any
    /// more, and the refresh grants and consents for it end.
    pub(crate) async fn delete_resource(
        &self,
        operator: &Operator,
        id: ResourceId,
        reason: Reason,
    ) -> Result<(), AdminError> {
        self.edit_resource(operator, id, ResourceEdit::Delete, reason)
            .await
    }
}

//! `/developers/resources`: the APIs the account registered as resources, and
//! one resource's page, with its name, its scopes and the clients it admits. A
//! resource's page reads it first, so another account's or an unknown one is
//! missing before anything else happens.

use crate::{
    account::{AccountError, Owner},
    resources::{NewResource, OwnedResource, OwnedResourceError, ResourceEdit},
    routes::page::developers::{self, HOME, RESOURCE, RESOURCES},
    web::{
        Failure, WebApplication,
        account::{AccountPage, Applied, Reader},
    },
};
use sourceorigin_ui::{Crumb, Messages, Notice, ResourceChange, ResourcesChange, Section, Title};
use topcoat::{
    context::Cx,
    router::{StatusCode, path_param, response::Response},
    view::{Child, view},
};

/// The account's resources, and registering one.
pub(in crate::web) struct Resources;

impl AccountPage for Resources {
    const SECTION: Section = Section::Developers;
    const PATH: &'static str = RESOURCES;
    const TITLE: &'static str = Title::Resources.as_str();
    type Change = ResourcesChange;
    type Row = ();

    fn reopened(_: &ResourcesChange) {}

    /// A registered resource goes on to its page, where its scopes are defined.
    async fn apply(
        web: &WebApplication,
        _: &Cx,
        owner: Owner,
        ResourcesChange::Register {
            identifier,
            display_name,
        }: ResourcesChange,
    ) -> Result<Applied<()>, AccountError> {
        let new =
            NewResource::typed(&identifier, &display_name).map_err(OwnedResourceError::from)?;
        let id = web.account.create_resource(&owner, &new).await?;
        Ok(Applied::Moved {
            to: developers::of(RESOURCE, &id),
            notice: Notice::ResourceRegistered,
        })
    }

    async fn render(
        web: &WebApplication,
        cx: &Cx,
        reader: &Reader,
        (): (),
        outcome: (StatusCode, Messages<'_>),
    ) -> Result<Response, Failure> {
        let resources = web.account.resources(reader.owner()).await?;
        let rows: Vec<_> = resources
            .iter()
            .map(|resource| (resource, developers::of(RESOURCE, &resource.id)))
            .collect();
        let page = view! { cx =>
            sourceorigin_ui::owned_resources(
                page: RESOURCES,
                home: HOME,
                rows: &rows,
                confirmed: reader.is_recent(),
            )
        };
        let head = Child::default();
        web.console_page(cx, Self::TITLE, reader, outcome, head, Child::new(page))
            .await
    }
}

/// One of the account's resources, and every change to it.
pub(in crate::web) struct Resource;

impl AccountPage for Resource {
    const SECTION: Section = Section::Developers;
    const PATH: &'static str = RESOURCE;
    const TITLE: &'static str = Title::Resource.as_str();
    type Change = ResourceChange;
    type Row = ();

    fn reopened(_: &ResourceChange) {}

    /// Every change needs a passkey confirmation, which the service checks under
    /// the account lock. A deleted resource goes back to the resources.
    async fn apply(
        web: &WebApplication,
        cx: &Cx,
        owner: Owner,
        change: ResourceChange,
    ) -> Result<Applied<()>, AccountError> {
        let resource = web.resource(cx, owner).await?;
        let edit = match change {
            ResourceChange::Rename { display_name } => ResourceEdit::rename(&display_name),
            ResourceChange::DefineScope { name, description } => {
                ResourceEdit::put_scope(&name, &description)
            }
            ResourceChange::RemoveScope { name } => Ok(ResourceEdit::RemoveScope(name)),
            ResourceChange::Grant { client, scopes } => ResourceEdit::grant(&client, &scopes),
            ResourceChange::Revoke { client } => ResourceEdit::revoke(&client),
            ResourceChange::Delete {} => Ok(ResourceEdit::Delete),
        }
        .map_err(OwnedResourceError::from)?;
        let deleted = matches!(edit, ResourceEdit::Delete);
        web.account.edit_resource(&resource, edit).await?;
        Ok(if deleted {
            Applied::Moved {
                to: RESOURCES.to_owned(),
                notice: Notice::ResourceDeleted,
            }
        } else {
            Applied::Done(Notice::ResourceChanged)
        })
    }

    async fn render(
        web: &WebApplication,
        cx: &Cx,
        reader: &Reader,
        (): (),
        outcome: (StatusCode, Messages<'_>),
    ) -> Result<Response, Failure> {
        let resource = web.resource(cx, reader.owner().clone()).await?;
        let detail = resource.detail();
        let here = developers::of(RESOURCE, &detail.resource.id);
        let page = view! { cx =>
            sourceorigin_ui::resource_settings(
                page: here.as_str(),
                home: RESOURCES,
                detail: detail,
                confirmed: reader.is_recent(),
            )
        };
        let title = Crumb::Name(&detail.resource.display_name);
        let head = Child::default();
        web.console_page(cx, title, reader, outcome, head, Child::new(page))
            .await
    }
}

impl WebApplication {
    /// The resource the page's path names, as `owner` owns it. Another account's
    /// or an unknown one is not found.
    async fn resource(&self, cx: &Cx, owner: Owner) -> Result<OwnedResource, AccountError> {
        self.account
            .owned_resource(owner, path_param::<super::Id>(cx))
            .await
    }
}

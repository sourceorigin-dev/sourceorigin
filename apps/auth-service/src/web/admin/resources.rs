//! The APIs registered as resources: the list, one resource's page, and the
//! changes to it, as the `resource` command makes them: registering and deleting
//! one, renaming it, defining and removing its scopes, and granting and revoking
//! a client's access.

use super::{
    AdminApplication as App, Administered, ReasonForm, ReasonPage, Refusal,
    clients::{self, Client},
};
use crate::{
    Reason,
    resources::{InvalidResourceInput, NewResource as Registered, ResourceEdit},
};
use serde::{Deserialize, Serialize};
use sourceorigin_common::{ClientId, ResourceId};
use sourceorigin_ui::{Action, AdminResourceRow, Change, Crumb, Query, ResourcePage, Target};
use topcoat::{
    Result,
    context::Cx,
    router::{
        StatusCode, content::Form, href, path_param, query_params, response::Response, route,
    },
    view::view,
};

path_param!(pub resource: ResourceId, error = not_found);

/// The form registering a resource: its identifier and name as typed, which the
/// dashboard checks, so a malformed one is explained beside the form.
#[derive(Clone, Deserialize)]
struct NewResource {
    /// The identifier, an absolute https URI.
    identifier: String,
    /// The name consent screens show.
    display_name: String,
}

/// The form renaming a resource.
#[derive(Clone, Deserialize)]
struct Rename {
    /// The new name.
    display_name: String,
}

/// The form defining a scope of a resource.
#[derive(Clone, Deserialize)]
struct Scope {
    /// The scope, as clients request it.
    name: String,
    /// What it lets a client do, as the consent screen says it.
    description: String,
}

/// The form granting a client access to a resource.
#[derive(Clone, Deserialize)]
struct Grant {
    /// The client's ID.
    client: String,
    /// The scopes it may request, separated by spaces.
    #[serde(default)]
    scopes: String,
}

/// The query of [`remove_scope`]: the scope to remove.
#[query_params(error = bad_request)]
#[derive(Serialize)]
struct ScopeQuery {
    /// The scope, as defined.
    scope: String,
}

impl Query for ScopeQuery {}

/// The query of [`revoke`]: the client whose access ends.
#[query_params(error = bad_request)]
#[derive(Serialize)]
struct ClientQuery {
    /// The client.
    client: ClientId,
}

impl Query for ClientQuery {}

/// `/resources`: every resource, each linking its page.
#[route(GET "/resources")]
pub(super) async fn list(cx: &Cx) -> Result<Response> {
    let admin = App::admin(cx).await?;
    let resources = App::of(cx).service.list_resources(&admin.operator).await?;
    let rows: Vec<_> = resources
        .iter()
        .map(|resource| AdminResourceRow {
            resource,
            href: href!(detail, Resource(resource.id)).resolve(cx),
        })
        .collect();
    let (rows, new) = (&rows, &href!(create).resolve(cx));
    let page = view! { cx => sourceorigin_ui::resource_list(rows: rows, new: new) };
    App::render(cx, StatusCode::OK, "Resources", page).await
}

/// `/resources/new`: registers a resource operators manage.
#[route([GET, POST] "/resources/new")]
pub(super) async fn create(
    cx: &Cx,
    form: Option<Form<Administered<NewResource>>>,
) -> Result<Response> {
    let admin = App::confirmed(cx).await?;
    let service = &App::of(cx).service;
    let back = href!(list).resolve(cx);
    let refused = App::submit_reason(cx, form, back.clone(), async |new: NewResource, reason| {
        let new = Registered::typed(&new.identifier, &new.display_name)?;
        service
            .create_resource(&admin.operator, &new.identifier, &new.name, reason)
            .await
            .map(drop)
            .map_err(Refusal::from)
    })
    .await?;
    let (identifier, name, reason) = refused.as_ref().map_or(("", "", ""), |refused| {
        let new = &refused.form.change;
        (
            new.identifier.as_str(),
            new.display_name.as_str(),
            refused.reason(),
        )
    });
    let refusal = refused.as_deref();
    let page = view! { cx =>
        sourceorigin_ui::dashboard_change(
            change: Change::RegisterResource,
            back: back.as_str(),
            refusal: refusal,
            reason: reason,
            <label>
                "Identifier"
                <input name="identifier" type="url" required="" maxlength="255" autocomplete="off" placeholder="https://notes.example/api" value=(identifier) autofocus="">
            </label>
            <label>
                "Name"
                <input name="display_name" required="" maxlength="80" autocomplete="off" value=(name)>
            </label>
        )
    };
    App::render(cx, StatusCode::OK, Change::RegisterResource.title(), page).await
}

/// `/resources/{resource}`: one resource, with its scopes, the clients it admits
/// and the changes to each.
#[route(GET "/resources/{resource}")]
pub(super) async fn detail(cx: &Cx) -> Result<Response> {
    let admin = App::admin(cx).await?;
    let id = *path_param::<Resource>(cx)?;
    let found = App::of(cx)
        .service
        .resource_detail(&admin.operator, id)
        .await?;
    let inline = admin.operator.is_confirmed(super::FORM_TIME);
    let action = |page: String, query: &dyn ErasedQuery| -> Result<Action> {
        Ok(Action {
            href: query.link(&page)?,
            inline,
        })
    };
    let scopes = found
        .scopes
        .iter()
        .map(|scope| {
            let query = ScopeQuery {
                scope: scope.name.clone(),
            };
            let page = href!(remove_scope, Resource(id)).resolve(cx);
            Ok((scope, action(page, &query)?))
        })
        .collect::<Result<_>>()?;
    let access = found
        .access
        .iter()
        .map(|access| {
            let query = ClientQuery {
                client: access.client_id.clone(),
            };
            let page = href!(revoke, Resource(id)).resolve(cx);
            let client = href!(clients::detail, Client(access.client_id.clone())).resolve(cx);
            Ok((access, client, action(page, &query)?))
        })
        .collect::<Result<_>>()?;
    let shown = ResourcePage {
        detail: &found,
        owner: (found.resource.owner_id).map(|owner| App::account(cx, owner)),
        rename: href!(rename, Resource(id)).resolve(cx),
        define: href!(define_scope, Resource(id)).resolve(cx),
        grant: href!(grant, Resource(id)).resolve(cx),
        delete: href!(delete, Resource(id)).resolve(cx),
        scopes,
        access,
    };
    let page = view! { cx => sourceorigin_ui::resource_page(page: &shown) };
    let title = Crumb::Name(&found.resource.display_name);
    App::render(cx, StatusCode::OK, title, page).await
}

/// `/resources/{resource}/rename`: gives the resource another name.
#[route([GET, POST] "/resources/{resource}/rename")]
pub(super) async fn rename(cx: &Cx, form: Option<Form<Administered<Rename>>>) -> Result<Response> {
    let (back, refused) = App::edit_resource(cx, form, |rename: Rename| {
        ResourceEdit::rename(&rename.display_name)
    })
    .await?;
    let (name, reason) = refused.as_ref().map_or(("", ""), |refused| {
        (refused.form.change.display_name.as_str(), refused.reason())
    });
    let (refusal, target) = (refused.as_deref(), App::target(cx).await);
    let page = view! { cx =>
        sourceorigin_ui::dashboard_change(
            change: Change::RenameResource,
            target: target,
            back: back.as_str(),
            refusal: refusal,
            reason: reason,
            <label>
                "Name"
                <input name="display_name" required="" maxlength="80" autocomplete="off" value=(name) autofocus="">
            </label>
        )
    };
    App::render(cx, StatusCode::OK, Change::RenameResource.title(), page).await
}

/// `/resources/{resource}/scopes/define`: defines a scope of the resource, or
/// describes it anew.
#[route([GET, POST] "/resources/{resource}/scopes/define")]
pub(super) async fn define_scope(
    cx: &Cx,
    form: Option<Form<Administered<Scope>>>,
) -> Result<Response> {
    let (back, refused) = App::edit_resource(cx, form, |scope: Scope| {
        ResourceEdit::put_scope(&scope.name, &scope.description)
    })
    .await?;
    let (name, description, reason) = refused.as_ref().map_or(("", "", ""), |refused| {
        let scope = &refused.form.change;
        (
            scope.name.as_str(),
            scope.description.as_str(),
            refused.reason(),
        )
    });
    let (refusal, target) = (refused.as_deref(), App::target(cx).await);
    let page = view! { cx =>
        sourceorigin_ui::dashboard_change(
            change: Change::DefineScope,
            target: target,
            back: back.as_str(),
            refusal: refusal,
            reason: reason,
            <label>
                "Scope"
                <input name="name" required="" maxlength="64" autocomplete="off" placeholder="notes:read" value=(name) autofocus="">
            </label>
            <label>
                "Description"
                <input name="description" required="" maxlength="200" autocomplete="off" placeholder="Read your notes" value=(description)>
            </label>
        )
    };
    App::render(cx, StatusCode::OK, Change::DefineScope.title(), page).await
}

/// `/resources/{resource}/scopes/remove?scope=…`: removes a scope from the
/// resource, and from what every client may request of it.
#[route([GET, POST] "/resources/{resource}/scopes/remove")]
pub(super) async fn remove_scope(cx: &Cx, form: Option<Form<ReasonForm>>) -> Result<Response> {
    let scope = query_params::<ScopeQuery>(cx)?.scope.clone();
    let edit = ResourceEdit::RemoveScope(scope.clone());
    App::resource_reason_change(cx, form, Change::RemoveScope, Some(&scope), edit).await
}

/// `/resources/{resource}/access/grant`: lets a client ask for the resource's
/// tokens, with the scopes typed.
#[route([GET, POST] "/resources/{resource}/access/grant")]
pub(super) async fn grant(cx: &Cx, form: Option<Form<Administered<Grant>>>) -> Result<Response> {
    let (back, refused) = App::edit_resource(cx, form, |grant: Grant| {
        ResourceEdit::grant(&grant.client, &grant.scopes)
    })
    .await?;
    let (client, scopes, reason) = refused.as_ref().map_or(("", "", ""), |refused| {
        let grant = &refused.form.change;
        (
            grant.client.as_str(),
            grant.scopes.as_str(),
            refused.reason(),
        )
    });
    let (refusal, target) = (refused.as_deref(), App::target(cx).await);
    let page = view! { cx =>
        sourceorigin_ui::dashboard_change(
            change: Change::GrantAccess,
            target: target,
            back: back.as_str(),
            refusal: refusal,
            reason: reason,
            <label>
                "Client ID"
                <input name="client" required="" maxlength="64" autocomplete="off" value=(client) autofocus="">
            </label>
            <label>
                "Scopes"
                <input name="scopes" autocomplete="off" placeholder="notes:read notes:write" value=(scopes) aria-describedby="scopes-help">
            </label>
            <small class="field-help" id="scopes-help">
                "The resource's scopes the client may request, separated by spaces. None: its tokens for the resource carry only Origin Auth's own scopes."
            </small>
        )
    };
    App::render(cx, StatusCode::OK, Change::GrantAccess.title(), page).await
}

/// `/resources/{resource}/access/revoke?client=…`: ends a client's access to the
/// resource, with its refresh tokens for it.
#[route([GET, POST] "/resources/{resource}/access/revoke")]
pub(super) async fn revoke(cx: &Cx, form: Option<Form<ReasonForm>>) -> Result<Response> {
    let client = query_params::<ClientQuery>(cx)?.client.clone();
    let edit = ResourceEdit::Revoke(client.clone());
    App::resource_reason_change(cx, form, Change::RevokeAccess, Some(client.as_str()), edit).await
}

/// `/resources/{resource}/delete`: deletes the resource for good.
#[route([GET, POST] "/resources/{resource}/delete")]
pub(super) async fn delete(cx: &Cx, form: Option<Form<ReasonForm>>) -> Result<Response> {
    App::resource_reason_change(cx, form, Change::DeleteResource, None, ResourceEdit::Delete).await
}

impl From<InvalidResourceInput> for Refusal {
    /// The form says what its field takes.
    fn from(error: InvalidResourceInput) -> Self {
        Self::Form(error.into())
    }
}

/// A query whose link is written without naming its type, so one closure writes
/// the links of the scope and client rows alike.
trait ErasedQuery {
    /// The link to `page` with this query.
    fn link(&self, page: &str) -> Result<String>;
}

impl<Q: Query> ErasedQuery for Q {
    fn link(&self, page: &str) -> Result<String> {
        Ok(self.on(page)?)
    }
}

impl App {
    /// Makes the edit `read` finds in a posted `form` to the resource the path
    /// names, once the dashboard is confirmed, then goes back to the resource's
    /// page. Returns that page, where Cancel leads too, and the form as refused,
    /// if it was, for the change's page to show again.
    async fn edit_resource<I: Clone>(
        cx: &Cx,
        form: Option<Form<Administered<I>>>,
        read: impl FnOnce(I) -> std::result::Result<ResourceEdit, InvalidResourceInput>,
    ) -> Result<(String, Option<super::Refused<Administered<I>>>)> {
        let admin = Self::confirmed(cx).await?;
        let service = &Self::of(cx).service;
        let id = *path_param::<Resource>(cx)?;
        let back = href!(detail, Resource(id)).resolve(cx);
        let refused = Self::submit_reason(cx, form, back.clone(), async |input: I, reason| {
            let edit = read(input)?;
            service
                .edit_resource(&admin.operator, id, edit, reason)
                .await
                .map_err(Refusal::from)
        })
        .await?;
        Ok((back, refused))
    }

    /// The page of `change`, which asks only why, making `edit` to the resource
    /// the path names: removing the scope or revoking the client `subject` names,
    /// then back to the resource's page, or deleting the resource, then back to
    /// the list.
    async fn resource_reason_change(
        cx: &Cx,
        form: Option<Form<ReasonForm>>,
        change: Change,
        subject: Option<&str>,
        edit: ResourceEdit,
    ) -> Result<Response> {
        let admin = Self::confirmed(cx).await?;
        let service = &Self::of(cx).service;
        let id = *path_param::<Resource>(cx)?;
        let back = href!(detail, Resource(id)).resolve(cx);
        let done = match edit {
            ResourceEdit::Delete => href!(list).resolve(cx),
            ResourceEdit::Rename(_)
            | ResourceEdit::PutScope(..)
            | ResourceEdit::RemoveScope(_)
            | ResourceEdit::Grant(..)
            | ResourceEdit::Revoke(_) => back.clone(),
        };
        let resource = id.to_string();
        let page = ReasonPage {
            target: subject.map(|name| Target {
                name,
                id: Some(&resource),
            }),
            ..ReasonPage::new(change, &back)
        };
        Self::reason_change(cx, form, page, done, async |reason: Reason| {
            service
                .edit_resource(&admin.operator, id, edit, reason)
                .await
        })
        .await
    }
}

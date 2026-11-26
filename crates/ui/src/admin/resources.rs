//! The APIs registered as resources: the list, and one resource's page, with the
//! scopes it defines and the clients it admits.

use super::{Action, Change, confirm, data_table, moment};
use crate::layout::page_head;
use sourceorigin_common::admin::{Resource, ResourceAccess, ResourceDetail, ScopeDefinition};
use topcoat::{
    Result,
    view::{View, component, view},
};

/// A resource on the dashboard's list.
#[derive(Clone, Debug)]
pub struct AdminResourceRow<'a> {
    /// The resource.
    pub resource: &'a Resource,
    /// Its page.
    pub href: String,
}

/// One resource's page: the resource, and where each change to it is made.
#[derive(Clone, Debug)]
pub struct ResourcePage<'a> {
    /// The resource, its scopes and the clients it admits.
    pub detail: &'a ResourceDetail,
    /// The page of the account that manages it; none for operators'.
    pub owner: Option<String>,
    /// The page renaming it.
    pub rename: String,
    /// The page defining a scope.
    pub define: String,
    /// The page granting a client access.
    pub grant: String,
    /// The page deleting it.
    pub delete: String,
    /// Each scope it defines, with the change removing it.
    pub scopes: Vec<(&'a ScopeDefinition, Action)>,
    /// Each client it admits, with the client's page and the change revoking its
    /// access.
    pub access: Vec<(&'a ResourceAccess, String, Action)>,
}

/// The resources, each linking its page, with the page registering one at `new`.
#[component]
pub async fn resource_list<'a>(
    /// The resources.
    rows: &'a [AdminResourceRow<'a>],
    /// Where a resource is registered.
    new: &'a str,
) -> Result<impl View> {
    Ok(view! {
        page_head(
            title: "Resources",
            lead: "APIs that access tokens are issued for, each with its own scopes and the clients it admits.".to_owned(),
            actions: view! {
                <a data-confirm-first="" class="button" href=(new)>"Register a resource"</a>
            },
        )
        <section class="card">
            data_table(
                headings: &["name", "identifier", "managed by", "registered"],
                empty: "No resource is registered.",
                filled: !rows.is_empty(),
                for row in rows {
                    let resource = row.resource;
                    <tr>
                        <td><a href=(row.href.as_str())>(resource.display_name.as_str())</a></td>
                        <td><code>(resource.identifier.as_str())</code></td>
                        <td>(if resource.owner_id.is_some() { "An account" } else { "Operators" })</td>
                        <td>moment(at: resource.created_at)</td>
                    </tr>
                }
            )
        </section>
    })
}

/// One resource: what it is, the scopes it defines and the clients that may ask
/// for its tokens, each with its change.
#[component]
pub async fn resource_page<'a>(
    /// The resource and its changes.
    page: &'a ResourcePage<'a>,
) -> Result<impl View> {
    let resource = &page.detail.resource;
    Ok(view! {
        page_head(
            title: resource.display_name.as_str(),
            lead: "An API that access tokens are issued for.".to_owned(),
            actions: view! {
                <a data-confirm-first="" class="button secondary" href=(page.rename.as_str())>"Rename"</a>
                <a data-confirm-first="" class="button danger" href=(page.delete.as_str())>"Delete"</a>
            },
        )
        <section class="card" aria-labelledby="resource-heading">
            <header><h2 id="resource-heading">"Resource"</h2></header>
            <div class="body">
                <dl class="facts">
                    <dt>"Identifier"</dt><dd><code>(resource.identifier.as_str())</code></dd>
                    <dt>"ID"</dt><dd><code>(resource.id.to_string())</code></dd>
                    <dt>"Managed by"</dt>
                    <dd>
                        if let Some(owner) = &page.owner {
                            <a href=(owner.as_str())>"An account"</a>
                        } else {
                            "Operators"
                        }
                    </dd>
                    <dt>"Registered"</dt><dd>moment(at: resource.created_at)</dd>
                    <dt>"Changed"</dt><dd>moment(at: resource.updated_at)</dd>
                </dl>
            </div>
        </section>
        <section class="card" aria-labelledby="scopes-heading">
            <header>
                <h2 id="scopes-heading">"Scopes"</h2>
                <a data-confirm-first="" class="button secondary small" href=(page.define.as_str())>"Define a scope"</a>
            </header>
            if page.scopes.is_empty() {
                <p class="empty">"It defines no scope: a client's tokens for it carry only Origin Auth's own scopes."</p>
            } else {
                <ul class="list">
                    for (scope, remove) in &page.scopes {
                        <li>
                            <div class="text">
                                <strong><code>(scope.name.as_str())</code></strong>
                                <small>(scope.description.as_str())</small>
                            </div>
                            confirm(action: remove, change: Change::RemoveScope, subject: scope.name.as_str())
                        </li>
                    }
                </ul>
            }
        </section>
        <section class="card" aria-labelledby="access-heading">
            <header>
                <h2 id="access-heading">"Clients with access"</h2>
                <a data-confirm-first="" class="button secondary small" href=(page.grant.as_str())>"Grant a client access"</a>
            </header>
            if page.access.is_empty() {
                <p class="empty">"No client may ask for its tokens."</p>
            } else {
                <ul class="list">
                    for (access, client, revoke) in &page.access {
                        <li>
                            <div class="text">
                                <a href=(client.as_str())><strong>(if access.client_name.is_empty() { access.client_id.as_str() } else { access.client_name.as_str() })</strong></a>
                                <small>
                                    <code>(access.client_id.as_str())</code>
                                    " · "
                                    if access.scopes.is_empty() {
                                        "no scope of its own"
                                    } else {
                                        <code>(access.scopes.join(" "))</code>
                                    }
                                    " · since " moment(at: access.granted_at)
                                </small>
                            </div>
                            confirm(action: revoke, change: Change::RevokeAccess, subject: access.client_id.as_str())
                        </li>
                    }
                </ul>
            }
        </section>
    })
}

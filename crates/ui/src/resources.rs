//! The developer console's resources: the APIs an account registered, which
//! access tokens are issued for, with the scopes each defines and the clients
//! each admits.

use crate::{
    form::{Precision, change_form, time},
    layout::page_head,
};
use serde::{Deserialize, Serialize};
use sourceorigin_common::admin::{Resource, ResourceDetail};
use topcoat::{
    Result,
    view::{View, component, view},
};

/// The change the resources page posts, named by its `action` field: registering
/// one, with what its form's inputs hold.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ResourcesChange {
    /// Registers a resource.
    Register {
        /// Its identifier, an absolute https URI, as typed.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        identifier: String,
        /// The name consent screens show, as typed.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        display_name: String,
    },
}

/// A change a resource's page posts, named by its `action` field. What the user
/// types is left empty in the change a form sends, whose inputs hold it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ResourceChange {
    /// Gives it another name.
    Rename {
        /// The new name, as typed.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        display_name: String,
    },
    /// Defines a scope, or describes it anew.
    DefineScope {
        /// The scope, as clients request it.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        name: String,
        /// What it lets a client do, as the consent screen says it.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        description: String,
    },
    /// Removes a scope.
    RemoveScope {
        /// The scope, as defined.
        name: String,
    },
    /// Lets a client ask for its tokens.
    Grant {
        /// The client's ID, as typed.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        client: String,
        /// The scopes it may request, separated by spaces, as typed.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        scopes: String,
    },
    /// Ends a client's access to it.
    Revoke {
        /// The client's ID.
        client: String,
    },
    /// Deletes it for good.
    Delete {},
}

/// The account's resources, each with its page in `rows`, and the form
/// registering one, which posts to `page`. Unless `confirmed`, a passkey confirms
/// the registration first.
#[component]
pub async fn owned_resources<'a>(
    /// Where the registration posts.
    page: &'a str,
    /// The console's home, the account's apps.
    home: &'a str,
    /// The resources, each with its page.
    rows: &'a [(&'a Resource, String)],
    /// Whether a passkey confirmed the session recently enough to register.
    confirmed: bool,
) -> Result<impl View> {
    let register = ResourcesChange::Register {
        identifier: String::new(),
        display_name: String::new(),
    };
    Ok(view! {
        page_head(
            title: "Your APIs",
            lead: "APIs you registered as resources: apps you admit get access tokens for them, with the scopes you define.".to_owned(),
            actions: view! { <a class="button secondary" href=(home)>"Your apps"</a> },
        )
        <section class="card" aria-labelledby="resources-heading">
            <header><h2 id="resources-heading">"APIs " <span class="count">(rows.len().to_string())</span></h2></header>
            <ul class="list">
                for (resource, href) in rows {
                    <li>
                        <div class="text">
                            <a href=(href.as_str())><strong>(resource.display_name.as_str())</strong></a>
                            <small>
                                <code>(resource.identifier.as_str())</code>
                                " · registered " time(at: resource.created_at, precision: Precision::Day)
                            </small>
                        </div>
                        <a class="button secondary small" href=(href.as_str())>"Open"</a>
                    </li>
                }
            </ul>
            if rows.is_empty() {
                <p class="empty">"You have no API yet. Register one to give apps access tokens for it."</p>
            }
        </section>
        <section class="card" aria-labelledby="register-heading">
            <header><h2 id="register-heading">"Register an API"</h2></header>
            <div class="body">
                change_form(
                    action: page,
                    change: register,
                    label: "Register API",
                    confirm: !confirmed,
                    <label>
                        "Identifier"
                        <input name="identifier" type="url" required="" maxlength="255" autocomplete="off" placeholder="https://notes.example/api" aria-describedby="identifier-help">
                    </label>
                    <small class="field-help" id="identifier-help">
                        "An https address your API checks its tokens against: their aud claim. Apps name it in the resource parameter. It can’t change later."
                    </small>
                    <label>
                        "Name"
                        <input name="display_name" required="" maxlength="80" autocomplete="off" aria-describedby="name-help">
                    </label>
                    <small class="field-help" id="name-help">"Users see it when an app asks for access to your API."</small>
                )
            </div>
        </section>
    })
}

/// One of the account's resources: its identifier and name, the scopes it
/// defines and the clients it admits, each with the form changing it, which
/// posts to `page`. Unless `confirmed`, a passkey confirms each change first.
#[component]
pub async fn resource_settings<'a>(
    /// Where the changes post.
    page: &'a str,
    /// The account's resources.
    home: &'a str,
    /// The resource, its scopes and the clients it admits.
    detail: &'a ResourceDetail,
    /// Whether a passkey confirmed the session recently enough to change it.
    confirmed: bool,
) -> Result<impl View> {
    let resource = &detail.resource;
    let rename = ResourceChange::Rename {
        display_name: String::new(),
    };
    Ok(view! {
        page_head(
            title: resource.display_name.as_str(),
            lead: "An API that access tokens are issued for.".to_owned(),
            actions: view! { <a class="button secondary" href=(home)>"Your APIs"</a> },
        )
        <section class="card" aria-labelledby="resource-heading">
            <header><h2 id="resource-heading">"API"</h2></header>
            <div class="body">
                <dl class="facts">
                    <dt>"Identifier"</dt><dd><code>(resource.identifier.as_str())</code></dd>
                    <dt>"Registered"</dt><dd>time(at: resource.created_at, precision: Precision::Day)</dd>
                </dl>
                change_form(
                    action: page,
                    change: rename,
                    label: "Rename",
                    class: "secondary",
                    confirm: !confirmed,
                    <label>
                        "Name"
                        <input name="display_name" required="" maxlength="80" autocomplete="off" value=(resource.display_name.as_str())>
                    </label>
                )
            </div>
        </section>
        resource_scopes(page: page, detail: detail, confirmed: confirmed)
        resource_access(page: page, detail: detail, confirmed: confirmed)
        <section class="card" aria-labelledby="danger-heading">
            <header><h2 id="danger-heading">"Danger zone"</h2></header>
            <ul class="list">
                <li>
                    <div class="text">
                        <strong>"Delete this API"</strong>
                        <small>"No app gets tokens for it any more, and every refresh token and consent for it ends. This cannot be undone."</small>
                    </div>
                    change_form(
                        action: page,
                        change: ResourceChange::Delete {},
                        label: "Delete",
                        class: "danger small",
                        confirm: !confirmed,
                    )
                </li>
            </ul>
        </section>
    })
}

/// The scopes `detail` defines, each with the form removing it, and the form
/// defining one. They post to `page`.
#[component]
async fn resource_scopes<'a>(
    /// Where the changes post.
    page: &'a str,
    /// The resource and its scopes.
    detail: &'a ResourceDetail,
    /// Whether a passkey confirmed the session recently enough to change it.
    confirmed: bool,
) -> Result<impl View> {
    let define = ResourceChange::DefineScope {
        name: String::new(),
        description: String::new(),
    };
    Ok(view! {
        <section class="card" aria-labelledby="scopes-heading">
            <header><h2 id="scopes-heading">"Scopes " <span class="count">(detail.scopes.len().to_string())</span></h2></header>
            <p class="help">"What an app can ask to do with your API. The consent screen shows each description to the user."</p>
            <ul class="list">
                for scope in &detail.scopes {
                    <li>
                        <div class="text">
                            <strong><code>(scope.name.as_str())</code></strong>
                            <small>(scope.description.as_str())</small>
                        </div>
                        change_form(
                            action: page,
                            change: ResourceChange::RemoveScope { name: scope.name.clone() },
                            label: "Remove",
                            class: "danger small",
                            aria_label: format!("Remove the scope {}", scope.name),
                            confirm: !confirmed,
                        )
                    </li>
                }
            </ul>
            if detail.scopes.is_empty() {
                <p class="empty">"No scope yet: an app’s tokens for this API carry only Origin Auth’s own scopes."</p>
            }
            <div class="body">
                change_form(
                    action: page,
                    change: define,
                    label: "Define scope",
                    class: "secondary",
                    confirm: !confirmed,
                    <label>
                        "Scope"
                        <input name="name" required="" maxlength="64" autocomplete="off" placeholder="notes:read">
                    </label>
                    <label>
                        "Description"
                        <input name="description" required="" maxlength="200" autocomplete="off" placeholder="Read your notes">
                    </label>
                )
            </div>
        </section>
    })
}

/// The clients `detail` admits, each with the form revoking its access, and the
/// form granting one access. They post to `page`.
#[component]
async fn resource_access<'a>(
    /// Where the changes post.
    page: &'a str,
    /// The resource and the clients it admits.
    detail: &'a ResourceDetail,
    /// Whether a passkey confirmed the session recently enough to change it.
    confirmed: bool,
) -> Result<impl View> {
    let grant = ResourceChange::Grant {
        client: String::new(),
        scopes: String::new(),
    };
    Ok(view! {
        <section class="card" aria-labelledby="access-heading">
            <header><h2 id="access-heading">"Apps with access " <span class="count">(detail.access.len().to_string())</span></h2></header>
            <p class="help">"Only the apps you list here get tokens for your API. Users still approve each app, unless Origin Auth trusts it."</p>
            <ul class="list">
                for access in &detail.access {
                    <li>
                        <div class="text">
                            <strong>(if access.client_name.is_empty() { access.client_id.as_str() } else { access.client_name.as_str() })</strong>
                            <small>
                                <code>(access.client_id.as_str())</code>
                                " · "
                                if access.scopes.is_empty() {
                                    "no scope of this API"
                                } else {
                                    <code>(access.scopes.join(" "))</code>
                                }
                            </small>
                        </div>
                        change_form(
                            action: page,
                            change: ResourceChange::Revoke { client: access.client_id.to_string() },
                            label: "Revoke",
                            class: "danger small",
                            aria_label: format!("Revoke the access of {}", access.client_id),
                            confirm: !confirmed,
                        )
                    </li>
                }
            </ul>
            if detail.access.is_empty() {
                <p class="empty">"No app has access yet."</p>
            }
            <div class="body">
                change_form(
                    action: page,
                    change: grant,
                    label: "Grant access",
                    class: "secondary",
                    confirm: !confirmed,
                    <label>
                        "Client ID"
                        <input name="client" required="" maxlength="64" autocomplete="off" aria-describedby="client-help">
                    </label>
                    <small class="field-help" id="client-help">"The app’s client ID, yours or someone else’s. Granting again replaces the scopes it may request."</small>
                    <label>
                        "Scopes"
                        <input name="scopes" autocomplete="off" placeholder="notes:read notes:write" aria-describedby="scopes-help">
                    </label>
                    <small class="field-help" id="scopes-help">"The scopes of this API the app may request, separated by spaces."</small>
                )
            </div>
        </section>
    })
}

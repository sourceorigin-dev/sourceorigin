//! The OIDC clients, as the `client` command manages them: the list, one
//! client's page, registering and changing one, enabling, disabling,
//! suspending and deleting it, and rotating a confidential client's secret. The
//! origins approved for a client's pages have their own pages, in [`origins`].
//! Applying a client a deployment manages stays the CLI's: see
//! [`AdminApplication::page`](super::AdminApplication::page).
//!
//! A page whose change could only be refused says so in place of its form: a
//! deleted client's change pages, approving an origin for its pages among them,
//! and a public client's secrets page, which has none, answer with the
//! registry's refusal.
//!
//! Every page of one client sits under its ID, its overview too, at
//! `/clients/{client}/overview`, so none shares a path with the registration
//! page at `/clients/new`, whatever the client's ID.

use super::{
    AdminApplication as App, Administered, ReasonForm, ReasonPage, Refusal,
    origins::{self, OriginQuery},
};
use crate::{
    AdminError, AdminService, ClientRegistryError, Operator, Reason,
    admin::SignedInAdmin,
    clients::{ClientSettings, InvalidSetting, OperatorSettings},
    security::ApiError,
};
use chrono::Utc;
use sourceorigin_common::{
    ClientId,
    admin::ClientDetail,
    clients::{ClientKind, ClientRegistration, ClientStatus},
};
use sourceorigin_ui::{
    Action, AdminClientRow, Change, ClientChanges, ClientDraft, ClientPage, ClientRow, Crumb,
    NewClientForm, OperatorDraft, Query, Registration, Revealed, SecretRow, SecretsChange,
    SecretsState,
};
use topcoat::{
    Result,
    context::Cx,
    router::{
        StatusCode, content::Form, error::see_other, href, path_param, request::uri,
        response::Response, route,
    },
    view::view,
};

path_param!(pub client: ClientId, error = not_found);

/// `/clients`: every OIDC client, deleted ones included, each linking its page.
#[route(GET "/clients")]
pub(super) async fn list(cx: &Cx) -> Result<Response> {
    let admin = App::admin(cx).await?;
    let clients = App::of(cx).service.list_clients(&admin.operator).await?;
    let rows = App::client_rows(cx, &clients);
    let (rows, new) = (&rows, &href!(create).resolve(cx));
    let page = view! { cx => sourceorigin_ui::client_list(rows: rows, new: new) };
    App::render(cx, StatusCode::OK, "Clients", page).await
}

/// `/clients/{client}/overview`: everything about one client, its secrets' lifetimes and
/// approved origins, and the changes the dashboard may make to it. Just after the
/// client's registration, it shows its first secret instead, once.
#[route(GET "/clients/{client}/overview")]
pub(super) async fn detail(cx: &Cx) -> Result<Response> {
    let admin = App::admin(cx).await?;
    let service = &App::of(cx).service;
    let id = path_param::<Client>(cx)?.clone();
    if let Some(secret) = App::revealed(cx, admin).map(Revealed::new) {
        let (secret, page) = (&secret, &href!(detail, Client(id)).resolve(cx));
        let page = view! { cx =>
            sourceorigin_ui::client_created(secret: secret, page: page.as_str())
        };
        return App::render(cx, StatusCode::OK, "Client registered", page).await;
    }
    let found = service.client_detail(&admin.operator, &id).await?;
    let approved = service.list_related_origins(&admin.operator).await?;
    let client = &found.client;
    let row = ClientRow::from(&found);
    let page = || Client(id.clone());
    let secrets: Vec<_> = found.secrets.iter().map(SecretRow::from).collect();
    let origins = approved
        .iter()
        .filter(|approved| approved.client_id == id)
        .map(|approved| {
            let query = OriginQuery {
                origin: approved.origin.clone(),
            };
            let remove = query.on(&href!(origins::remove, page()).resolve(cx))?;
            Ok((approved.origin.as_str(), remove))
        })
        .collect::<Result<_>>()?;
    let approve = if client.deleted_at.is_some() {
        Err("Deleted clients sign nobody in.")
    } else if client.trusted {
        Ok(href!(origins::approve, page()).resolve(cx))
    } else {
        Err("Only trusted clients' pages sign users in themselves.")
    };
    let inline = admin.operator.is_confirmed(super::FORM_TIME);
    let action = |href: String| Action { href, inline };
    let changes = client.deleted_at.is_none().then(|| ClientChanges {
        settings: href!(edit, page()).resolve(cx),
        status: match row.status {
            ClientStatus::Deleted { .. } | ClientStatus::Suspended { .. } => None,
            ClientStatus::Enabled => Some(action(href!(disable, page()).resolve(cx))),
            ClientStatus::Disabled => Some(action(href!(enable, page()).resolve(cx))),
        },
        suspension: action(match row.status {
            ClientStatus::Suspended { .. } => href!(unsuspend, page()).resolve(cx),
            ClientStatus::Deleted { .. } | ClientStatus::Enabled | ClientStatus::Disabled => {
                href!(suspend, page()).resolve(cx)
            }
        }),
        delete: action(href!(delete, page()).resolve(cx)),
        secret: (found.kind == ClientKind::Confidential).then(|| href!(rotate, page()).resolve(cx)),
    });
    let shown = ClientPage {
        client: row,
        trusted: client.trusted,
        audiences: &client.allowed_audiences,
        owner: client
            .owner_id
            .map(|owner| (owner, App::account(cx, owner))),
        created_at: found.created_at,
        updated_at: found.updated_at,
        secrets: &secrets,
        origins,
        approve,
        changes,
    };
    let shown = &shown;
    let page = view! { cx =>
        sourceorigin_ui::client_page(page: shown, now: Utc::now())
    };
    Box::pin(App::render(
        cx,
        StatusCode::OK,
        Crumb::Name(App::title(&found)),
        page,
    ))
    .await
}

/// `/clients/new`: registers a client operators manage, then goes on to its page,
/// which shows a confidential one's first secret, once.
#[route([GET, POST] "/clients/new")]
pub(super) async fn create(cx: &Cx, form: Option<Form<NewClientForm>>) -> Result<Response> {
    let admin = App::confirmed(cx).await?;
    let service = &App::of(cx).service;
    let created = App::change(cx, form, async |new: NewClientForm| {
        let reason = Reason::parse(new.settings.reason.clone())?;
        let registration = App::registration(new)?;
        let secret = service
            .create_client(&admin.operator, &registration, reason)
            .await?;
        Ok::<_, Refusal>((registration.id, secret))
    })
    .await?;
    let (shown, refusal) = match created {
        Some(Ok((id, secret))) => {
            let page = href!(detail, Client(id)).resolve(cx);
            return match secret {
                Some(secret) => App::reveal(cx, admin, page, secret.expose()),
                None => Err(see_other(page).into()),
            };
        }
        Some(Err(refused)) => (refused.form.clone(), Some(refused.message)),
        None => (App::new_client(), None),
    };
    let registration = Registration::New {
        id: &shown.id,
        kind: shown.kind(),
    };
    Box::pin(App::registration_page(
        cx,
        registration,
        &shown.settings,
        refusal.as_deref(),
    ))
    .await
}

/// `/clients/{client}/settings`: changes a client's registration, every setting
/// but its ID and kind, then goes back to its page.
#[route([GET, POST] "/clients/{client}/settings")]
pub(super) async fn edit(cx: &Cx, form: Option<Form<OperatorDraft>>) -> Result<Response> {
    let admin = App::confirmed(cx).await?;
    let service = &App::of(cx).service;
    let found = App::changing(cx, admin).await?;
    let id = found.client.id.clone();
    let back = href!(detail, Client(id.clone())).resolve(cx);
    let refusal = App::submit(cx, form, back.clone(), async |draft: OperatorDraft| {
        let reason = Reason::parse(draft.reason.clone())?;
        service
            .update_client(&admin.operator, &id, reason, |stored| {
                OperatorSettings::from(draft)
                    .registration(stored.id.clone(), stored.kind)
                    .map_err(Refusal::from)
            })
            .await
            .map(drop)
    })
    .await?;
    let shown = match &refusal {
        Some(refused) => refused.form.clone(),
        None => OperatorDraft::of(&ClientRow::from(&found), found.client.trusted),
    };
    let registration = Registration::Edit {
        name: App::title(&found),
    };
    Box::pin(App::registration_page(
        cx,
        registration,
        &shown,
        refusal.as_deref(),
    ))
    .await
}

/// `/clients/{client}/secret`: a confidential client's secrets, and rotating
/// them. A rotation comes back here, which shows the new secret once.
#[route([GET, POST] "/clients/{client}/secret")]
pub(super) async fn rotate(
    cx: &Cx,
    form: Option<Form<Administered<SecretsChange>>>,
) -> Result<Response> {
    let admin = App::admin(cx).await?;
    let rotated = App::revealed(cx, admin).map(Revealed::new);
    if rotated.is_none() {
        App::confirm(cx, admin)?;
    }
    let service = &App::of(cx).service;
    let found = App::changing(cx, admin).await?;
    let id = found.client.id.clone();
    if found.kind == ClientKind::Public {
        return Err(AdminError::from(ClientRegistryError::Public(id)).into());
    }
    let here = href!(rotate, Client(id.clone())).resolve(cx);
    let changed = App::change(cx, form, async |rotation| {
        let SecretsChange::Rotate { grace } = rotation.change;
        let reason = Reason::parse(rotation.reason)?;
        service
            .rotate_client_secret(
                &admin.operator,
                &id,
                grace.unwrap_or_default().into(),
                reason,
            )
            .await
    })
    .await?;
    let (state, reason, refusal) = match changed {
        Some(Ok(secret)) => return App::reveal(cx, admin, here, secret.expose()),
        Some(Err(refused)) => {
            let SecretsChange::Rotate { grace } = refused.form.change;
            (
                SecretsState {
                    grace,
                    rotated: None,
                },
                refused.form.reason,
                Some(refused.message),
            )
        }
        None => (
            SecretsState {
                grace: None,
                rotated,
            },
            String::new(),
            None,
        ),
    };
    let secrets: Vec<_> = found.secrets.iter().map(SecretRow::from).collect();
    let (here, name, secrets, state, reason, refusal) = (
        &here,
        App::title(&found),
        &secrets,
        &state,
        &reason,
        refusal.as_deref(),
    );
    let page = view! { cx =>
        sourceorigin_ui::client_secrets(
            page: here.as_str(),
            name: name,
            secrets: secrets,
            now: Utc::now(),
            state: state,
            reason: reason.as_str(),
            refusal: refusal,
        )
    };
    Box::pin(App::render(cx, StatusCode::OK, "Client secrets", page)).await
}

/// A change to a client that only needs confirming, as its `client` command
/// makes it, each on its own page.
#[derive(Clone, Copy)]
enum Toggle {
    /// `client enable`.
    Enable,
    /// `client disable`.
    Disable,
    /// `client suspend`.
    Suspend,
    /// `client unsuspend`.
    Unsuspend,
    /// `client delete`.
    Delete,
}

impl Toggle {
    /// What it does, in the dashboard's words.
    const fn change(self) -> Change {
        match self {
            Self::Enable => Change::EnableClient,
            Self::Disable => Change::DisableClient,
            Self::Suspend => Change::SuspendClient,
            Self::Unsuspend => Change::UnsuspendClient,
            Self::Delete => Change::DeleteClient,
        }
    }

    /// Makes it to client `id`, as `operator`, for `reason`.
    async fn make(
        self,
        service: &AdminService,
        operator: &Operator,
        id: &ClientId,
        reason: Reason,
    ) -> Result<(), AdminError> {
        match self {
            Self::Enable => service.set_client_enabled(operator, id, true, reason).await,
            Self::Disable => {
                service
                    .set_client_enabled(operator, id, false, reason)
                    .await
            }
            Self::Suspend => {
                service
                    .set_client_suspended(operator, id, true, reason)
                    .await
            }
            Self::Unsuspend => {
                service
                    .set_client_suspended(operator, id, false, reason)
                    .await
            }
            Self::Delete => service.delete_client(operator, id, reason).await,
        }
    }

    /// Its page, for the client the path names: a `GET` asks to confirm it, and a
    /// `POST` makes it, then goes back to the client's page, or to the clients
    /// once the client is deleted.
    async fn page(self, cx: &Cx, form: Option<Form<ReasonForm>>) -> Result<Response> {
        let admin = App::confirmed(cx).await?;
        let service = &App::of(cx).service;
        let id = App::changing(cx, admin).await?.client.id;
        let back = href!(detail, Client(id.clone())).resolve(cx);
        let done = match self {
            Self::Delete => href!(list).resolve(cx),
            Self::Enable | Self::Disable | Self::Suspend | Self::Unsuspend => back.clone(),
        };
        let change = self.change();
        let page = ReasonPage::new(change, &back);
        App::reason_change(cx, form, page, done, async |reason| {
            self.make(service, &admin.operator, &id, reason).await
        })
        .await
    }
}

/// `/clients/{client}/enable`: lets a disabled client sign users in again.
#[route([GET, POST] "/clients/{client}/enable")]
pub(super) async fn enable(cx: &Cx, form: Option<Form<ReasonForm>>) -> Result<Response> {
    Toggle::Enable.page(cx, form).await
}

/// `/clients/{client}/disable`: stops a client signing users in.
#[route([GET, POST] "/clients/{client}/disable")]
pub(super) async fn disable(cx: &Cx, form: Option<Form<ReasonForm>>) -> Result<Response> {
    Toggle::Disable.page(cx, form).await
}

/// `/clients/{client}/suspend`: blocks a client, so that its owner cannot enable
/// it again.
#[route([GET, POST] "/clients/{client}/suspend")]
pub(super) async fn suspend(cx: &Cx, form: Option<Form<ReasonForm>>) -> Result<Response> {
    Toggle::Suspend.page(cx, form).await
}

/// `/clients/{client}/unsuspend`: lifts a client's suspension; it stays disabled.
#[route([GET, POST] "/clients/{client}/unsuspend")]
pub(super) async fn unsuspend(cx: &Cx, form: Option<Form<ReasonForm>>) -> Result<Response> {
    Toggle::Unsuspend.page(cx, form).await
}

/// `/clients/{client}/delete`: deletes a client for good.
#[route([GET, POST] "/clients/{client}/delete")]
pub(super) async fn delete(cx: &Cx, form: Option<Form<ReasonForm>>) -> Result<Response> {
    Toggle::Delete.page(cx, form).await
}

impl From<OperatorDraft> for OperatorSettings {
    /// The settings as the form sent them, with every scope chosen.
    fn from(operator: OperatorDraft) -> Self {
        let allowed_scopes = operator.scopes();
        Self {
            settings: ClientSettings {
                allowed_scopes,
                ..ClientSettings::from(operator.draft)
            },
            trusted: operator.trusted.0,
        }
    }
}

impl From<InvalidSetting> for Refusal {
    /// The form says which setting is invalid, and why.
    fn from(invalid: InvalidSetting) -> Self {
        Self::Form(ApiError::bad_request(invalid.to_string()))
    }
}

impl From<ClientRegistryError> for Refusal {
    /// What the dashboard does about the registry refusing a change.
    fn from(error: ClientRegistryError) -> Self {
        AdminError::from(error).into()
    }
}

impl App {
    /// The client the path names, for a page that changes it. A deleted client
    /// changes no more, so each of its change pages answers at once with the
    /// registry's refusal, as its form would once posted.
    pub(super) async fn changing(cx: &Cx, admin: &SignedInAdmin) -> Result<ClientDetail> {
        let id = path_param::<Client>(cx)?;
        let found = Self::of(cx)
            .service
            .client_detail(&admin.operator, id)
            .await?;
        if found.client.deleted_at.is_some() {
            return Err(AdminError::from(ClientRegistryError::Deleted(id.clone())).into());
        }
        Ok(found)
    }

    /// `clients` as a list shows them, each linking its page.
    pub(super) fn client_rows<'a>(
        cx: &Cx,
        clients: &'a [sourceorigin_common::admin::Client],
    ) -> Vec<AdminClientRow<'a>> {
        clients
            .iter()
            .map(|client| AdminClientRow {
                client,
                href: href!(detail, Client(client.id.clone())).resolve(cx),
            })
            .collect()
    }

    /// The name of client `found`'s pages: its own, or its ID once deleted,
    /// which erases its name.
    pub(super) fn title(found: &ClientDetail) -> &str {
        let client = &found.client;
        if client.display_name.is_empty() {
            client.id.as_str()
        } else {
            &client.display_name
        }
    }

    /// A new client's form, as it starts: the defaults of the kind it starts at.
    fn new_client() -> NewClientForm {
        let mut new = NewClientForm::default();
        new.settings.draft = ClientDraft::new_client(NewClientForm::DEFAULT_KIND);
        new
    }

    /// The registration `new` asks for, its ID and every setting validated.
    fn registration(new: NewClientForm) -> Result<ClientRegistration, Refusal> {
        let kind = new.kind();
        let id: ClientId =
            new.id.trim().parse().map_err(|error| {
                Refusal::Form(ApiError::bad_request(format!("client ID: {error}")))
            })?;
        Ok(OperatorSettings::from(new.settings).registration(id, kind)?)
    }

    /// Answers with the page of `registration`, its form showing `settings`, and
    /// why the last attempt was refused, if it was.
    async fn registration_page(
        cx: &Cx,
        registration: Registration<'_>,
        settings: &OperatorDraft,
        refusal: Option<&str>,
    ) -> Result<Response> {
        let here = uri(cx).path().to_owned();
        let (title, here) = (registration.title(), &here);
        let page = view! { cx =>
            sourceorigin_ui::client_registration(
                registration: registration,
                settings: settings,
                action: here.as_str(),
                refusal: refusal,
            )
        };
        Box::pin(App::render(cx, StatusCode::OK, title, page)).await
    }
}

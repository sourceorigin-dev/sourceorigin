//! `/developers/clients/{id}`: one of the account's apps, its overview, insights,
//! settings and secrets. Each page reads the app first, so another account's or an unknown
//! one is missing before anything else happens.

use crate::{
    account::{AccountError, Owner},
    charted::Period,
    clients::{AppInsights, ClientPatch, ClientSettings, FLOOR, Floored},
    routes::{
        oidc::DISCOVERY,
        page::developers::{self, APP, APP_INSIGHTS, APP_SECRETS, APP_SETTINGS, HOME},
    },
    web::{
        Failure, WebApplication,
        account::{AccountPage, Applied, Reader, no_change},
    },
};
use chrono::Utc;
use sourceorigin_common::{ErrorChain, clients::ClientKind};
use sourceorigin_ui::{
    AppChange, AppSettingsState, AppTab, Charted, Checkbox, ClientRow, Connection, InsightsReport,
    Messages, Notice, Revealed, SecretRow, SecretsChange, SecretsState, Section, Title,
};
use topcoat::{
    context::Cx,
    router::{StatusCode, response::Response},
    view::{Child, view},
};

/// An app's overview: what its `OpenID` Connect library needs.
pub(in crate::web) struct Overview;

impl AccountPage for Overview {
    const SECTION: Section = Section::Developers;
    const PATH: &'static str = APP;
    const TITLE: &'static str = Title::App.as_str();
    type Row = ();
    no_change!();

    async fn render(
        web: &WebApplication,
        cx: &Cx,
        reader: &Reader,
        (): (),
        outcome: (StatusCode, Messages<'_>),
    ) -> Result<Response, Failure> {
        let app = web.read_app(cx, reader).await?;
        let client = app.client();
        let secrets = (client.kind == ClientKind::Confidential)
            .then(|| developers::of(APP_SECRETS, &client.id));
        let issuer = web.issuer.serialized();
        let discovery = web.issuer.join(DISCOVERY);
        let connection = Connection {
            issuer: &issuer,
            discovery: &discovery,
        };
        let row = ClientRow::from(client);
        let page = view! { cx =>
            sourceorigin_ui::overview(client: row, connection: connection, secrets: secrets.as_deref())
        };
        web.app_page(
            cx,
            AppTab::Overview,
            reader,
            client,
            outcome,
            Child::new(page),
        )
        .await
    }
}

/// How an app is used, over its last 30 whole days: how far its sign-ins get,
/// and the tokens it was issued. A ClickHouse that doesn't answer is said on the
/// page.
pub(in crate::web) struct Insights;

impl AccountPage for Insights {
    const SECTION: Section = Section::Developers;
    const PATH: &'static str = APP_INSIGHTS;
    const TITLE: &'static str = Title::App.as_str();
    type Row = ();
    no_change!();

    async fn render(
        web: &WebApplication,
        cx: &Cx,
        reader: &Reader,
        (): (),
        outcome: (StatusCode, Messages<'_>),
    ) -> Result<Response, Failure> {
        let app = web.read_app(cx, reader).await?;
        let page = match &web.analytics {
            None => Charted::Off,
            Some(analytics) => match app.insights(analytics, &web.noise).await {
                Ok(insights) => Charted::Shown(Self::report(insights)),
                Err(error) => {
                    tracing::warn!(error = %ErrorChain(&error), "an app's insights could not be read");
                    Charted::Unavailable
                }
            },
        };
        let view = view! { cx => sourceorigin_ui::app_insights(page: page, floor: FLOOR) };
        web.app_page(
            cx,
            AppTab::Insights,
            reader,
            app.client(),
            outcome,
            Child::new(view),
        )
        .await
    }
}

impl Insights {
    /// The page's rows of `insights`.
    fn report(insights: AppInsights) -> InsightsReport {
        InsightsReport {
            steps: insights
                .steps
                .into_iter()
                .map(|(step, count)| {
                    let shown = match count {
                        Floored::Few => None,
                        Floored::About(count) => Some(f64::from(count)),
                    };
                    (step, shown)
                })
                .collect(),
            days: insights
                .tokens
                .iter()
                .map(|tokens| tokens.label("%b %-d"))
                .collect(),
            exchanged: insights
                .tokens
                .iter()
                .map(|tokens| f64::from(tokens.exchanged))
                .collect(),
            refreshed: insights
                .tokens
                .iter()
                .map(|tokens| f64::from(tokens.refreshed))
                .collect(),
        }
    }
}

/// An app's settings, disabling and deleting it.
pub(in crate::web) struct AppSettings;

impl AccountPage for AppSettings {
    const SECTION: Section = Section::Developers;
    const PATH: &'static str = APP_SETTINGS;
    const TITLE: &'static str = Title::App.as_str();
    type Change = AppChange;
    type Row = AppSettingsState;

    fn reopened(change: &AppChange) -> AppSettingsState {
        match change {
            AppChange::Save { draft } => AppSettingsState {
                draft: Some(draft.clone()),
                ..AppSettingsState::default()
            },
            AppChange::Delete {} => AppSettingsState {
                deleting: Checkbox(true),
                draft: None,
            },
            AppChange::Disable {} | AppChange::Enable {} => AppSettingsState::default(),
        }
    }

    /// Every change needs a passkey confirmation, which the service checks under
    /// the account lock. A deleted app goes back to the apps.
    async fn apply(
        web: &WebApplication,
        cx: &Cx,
        owner: Owner,
        change: AppChange,
    ) -> Result<Applied<AppSettingsState>, AccountError> {
        let app = web.app(cx, owner).await?;
        let notice = match change {
            AppChange::Save { draft } => {
                let patch = ClientPatch::from(ClientSettings::from(draft));
                web.account.update_client(&app, patch).await?;
                Notice::ClientSaved
            }
            AppChange::Disable {} => {
                web.account.set_client_enabled(&app, false).await?;
                Notice::ClientDisabled
            }
            AppChange::Enable {} => {
                web.account.set_client_enabled(&app, true).await?;
                Notice::ClientEnabled
            }
            AppChange::Delete {} => {
                web.account.delete_client(&app).await?;
                return Ok(Applied::Moved {
                    to: HOME.to_owned(),
                    notice: Notice::ClientDeleted,
                });
            }
        };
        Ok(Applied::Done(notice))
    }

    async fn render(
        web: &WebApplication,
        cx: &Cx,
        reader: &Reader,
        state: AppSettingsState,
        outcome: (StatusCode, Messages<'_>),
    ) -> Result<Response, Failure> {
        let app = web.read_app(cx, reader).await?;
        let client = app.client();
        let page = developers::of(APP_SETTINGS, &client.id);
        let row = ClientRow::from(client);
        let view = view! { cx =>
            sourceorigin_ui::app_settings(
                page: &page,
                client: row,
                state: &state,
                confirmed: reader.is_recent(),
                error: outcome.1.field_error(),
            )
        };
        web.app_page(
            cx,
            AppTab::Settings,
            reader,
            client,
            outcome,
            Child::new(view),
        )
        .await
    }
}

/// A confidential app's secrets, and rotating them. A public app has none, so
/// its page is missing.
pub(in crate::web) struct AppSecrets;

impl AccountPage for AppSecrets {
    const SECTION: Section = Section::Developers;
    const PATH: &'static str = APP_SECRETS;
    const TITLE: &'static str = Title::App.as_str();
    type Change = SecretsChange;
    type Row = SecretsState;

    fn reopened(&SecretsChange::Rotate { grace }: &SecretsChange) -> SecretsState {
        SecretsState {
            grace,
            rotated: None,
        }
    }

    /// The new secret, which nothing shows again, shows in this answer.
    async fn apply(
        web: &WebApplication,
        cx: &Cx,
        owner: Owner,
        SecretsChange::Rotate { grace }: SecretsChange,
    ) -> Result<Applied<SecretsState>, AccountError> {
        let app = web.app(cx, owner).await?;
        let grace = grace.unwrap_or_default().into();
        let secret = web.account.rotate_client_secret(&app, grace).await?;
        Ok(Applied::Revealed(SecretsState {
            grace: None,
            rotated: Some(Revealed::new(secret.expose().to_owned())),
        }))
    }

    async fn render(
        web: &WebApplication,
        cx: &Cx,
        reader: &Reader,
        state: SecretsState,
        outcome: (StatusCode, Messages<'_>),
    ) -> Result<Response, Failure> {
        let app = web.read_app(cx, reader).await?;
        let client = app.client();
        if client.kind == ClientKind::Public {
            return Err(Failure::Missing);
        }
        let secrets = web.account.client_secrets(&app).await?;
        let rows: Vec<_> = secrets.iter().map(SecretRow::from).collect();
        let page = developers::of(APP_SECRETS, &client.id);
        let view = view! { cx =>
            sourceorigin_ui::app_secrets(
                page: &page,
                secrets: &rows,
                now: Utc::now(),
                state: &state,
                confirmed: reader.is_recent(),
            )
        };
        web.app_page(
            cx,
            AppTab::Secrets,
            reader,
            client,
            outcome,
            Child::new(view),
        )
        .await
    }
}

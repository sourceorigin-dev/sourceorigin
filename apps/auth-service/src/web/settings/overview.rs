//! `/settings`: the account at a glance.

use crate::{
    routes::page::settings,
    web::{
        Failure, WebApplication,
        account::{AccountPage, Reader, no_change},
        settings::HeldRows,
    },
};
use chrono::{Duration, Utc};
use sourceorigin_ui::{
    AccountOverview, ConnectedAppRow, DeviceRow, Messages, OverviewLinks, Section, Tab,
};
use topcoat::{
    context::Cx,
    router::{StatusCode, response::Response},
    view::{Child, view},
};

/// How many of the newest history entries the overview shows.
const ACTIVITY_SHOWN: usize = 6;

/// The overview page, which changes nothing.
pub(in crate::web) struct Overview;

impl Overview {
    /// Its tab.
    const TAB: Tab = Tab::Overview;
}

impl AccountPage for Overview {
    const SECTION: Section = Section::Account;
    const PATH: &'static str = settings::HOME;
    const TITLE: &'static str = Self::TAB.label();
    type Row = ();
    no_change!();

    async fn render(
        web: &WebApplication,
        cx: &Cx,
        reader: &Reader,
        (): (),
        outcome: (StatusCode, Messages<'_>),
    ) -> Result<Response, Failure> {
        let viewer = reader.viewer();
        let now = Utc::now();
        let sessions = web.account.sessions(&viewer).await?;
        let apps = web.account.apps(&viewer).await?;
        let activity = web
            .account
            .activity(&viewer, now - Duration::days(7))
            .await?;
        let history = web.account.history(&viewer, None).await?;
        let holdings = web.account.holdings(&viewer).await?;
        let unread = web.account.unread_notifications(&viewer).await?;
        let profile = web.account.profile(&viewer).await?;
        let providers = web.federation.provider_rows(&holdings.identities);
        let held = HeldRows::from(&holdings);
        let devices: Vec<_> = sessions.iter().map(DeviceRow::from).collect();
        let app_rows: Vec<_> = apps.iter().map(ConnectedAppRow::from).collect();
        let mut rows = history.rows();
        rows.truncate(ACTIVITY_SHOWN);
        let overview = AccountOverview {
            devices: &devices,
            apps: &app_rows,
            app_sign_ins_week: activity.app_sign_ins,
            security_events_week: activity.security_events,
            activity: &rows,
            holds: held.holds(reader.recovery()),
            providers: &providers,
            sends: web.email.smtp.is_some(),
            confirmed: reader.is_recent(),
            default_profile: profile.keeps_default(),
            now,
            unread,
        };
        let links = OverviewLinks {
            sign_in: settings::SIGN_IN,
            emails: settings::EMAILS,
            profile: settings::PROFILE,
            devices: settings::SESSIONS,
            history: settings::HISTORY,
            notifications: settings::NOTIFICATIONS,
        };
        let page = Child::new(view! { cx =>
            sourceorigin_ui::account_overview(overview: overview, links: links)
        });
        let targets = web
            .federation
            .authorization_endpoints()
            .map_err(topcoat::Error::from)?;
        web.settings_page(cx, Self::TAB, reader, outcome, &targets, page)
            .await
    }
}

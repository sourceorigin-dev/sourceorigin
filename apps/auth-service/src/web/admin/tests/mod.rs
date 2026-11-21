//! The admin dashboard through auth's two listeners, as a browser uses them: the
//! shared harness and fixtures here, and one file per flow.

use super::{AdminApplication, Attention, AttentionCache};
use crate::{
    AdminError, AdminService, AuthConfig, DashboardConfig, Reason,
    account::AccountService,
    admin::DashboardSignIn,
    assets::EmbeddedAssets,
    passkey::AdminProof,
    routes,
    security::{BrowserCookie, RequestContext},
    tests::{
        AUTH_ORIGIN, Browser, OWN_PAGE, Replicas, cookie, html, location, operator, parameter,
        policy, reason, redirect, until_blocked,
    },
};
use axum::{
    Router,
    http::{StatusCode, header},
    response::Response,
};
use chrono::{DateTime, Utc};
use sourceorigin_common::{
    AccountStatus, AuthenticationMethod, Handle, PictureId, UserId,
    admin::EventSearch,
    events::{Actor, DomainEvent},
    notifications::Notification,
};
use sourceorigin_repository::{Channel, Database, EventFilter, NewDelivery, SigningKeyState};
use sourceorigin_telemetry::{HttpTelemetry, Telemetry};
use sourceorigin_ui::HandoffQuery;
use std::net::SocketAddr;
use url::{Position, Url};

mod accounts;
mod announcements;
mod clients;
mod frame;
mod operations;
mod sessions;

/// The dashboard's origin in these scenarios.
const DASHBOARD: &str = "https://admin.sourceorigin.dev";
/// The reason field every change's form posts.
const REASON: (&str, &str) = ("reason", "Asked by email");

/// Auth, with a dashboard on its admin listener.
struct Harness {
    /// Auth's public listener.
    auth: Replicas,
    /// The admin listener.
    dashboard: Router,
    /// The scenario database.
    database: Database,
}

impl Harness {
    /// Builds auth and its dashboard over the test database.
    async fn new() -> Self {
        Self::of(AuthConfig::test(&AUTH_ORIGIN.parse().expect("auth origin"))).await
    }

    /// Builds auth as `config` sets it up, and its dashboard, over the test
    /// database.
    async fn of(config: AuthConfig) -> Self {
        let config = Self::with_dashboard(config);
        Self {
            auth: Replicas::of(&config).await,
            dashboard: Replicas::dashboard(&config).await,
            database: Replicas::database().await,
        }
    }

    /// Builds auth and its dashboard over a schema of their own, for a scenario
    /// no other may disturb, such as one on the current alerts, which a
    /// verification of the shared test database clears.
    async fn isolated() -> Self {
        let config =
            Self::with_dashboard(AuthConfig::test(&AUTH_ORIGIN.parse().expect("auth origin")));
        let database = Database::isolated(Replicas::database_config(5))
            .await
            .expect("isolated database");
        Self {
            auth: Replicas::over(&config, &database).await,
            dashboard: Replicas::dashboard_over(&config, database.clone()).await,
            database,
        }
    }

    /// `config` with a dashboard at [`DASHBOARD`].
    fn with_dashboard(mut config: AuthConfig) -> AuthConfig {
        config.dashboard = Some(DashboardConfig {
            listener: SocketAddr::from(([127, 0, 0, 1], 0)),
            origin: DASHBOARD.parse().expect("dashboard origin"),
        });
        config
    }

    /// A browser signed in to auth with a passkey, of a new administrator.
    async fn admin(&self) -> Browser {
        let browser = Browser::new_account(&self.database).await;
        self.grant(browser.user_id).await;
        browser
    }

    /// Makes `user_id` an administrator, from the CLI.
    async fn grant(&self, user_id: UserId) {
        AdminService::new(self.database.clone(), true)
            .grant_administrator(&operator(), user_id, reason())
            .await
            .expect("grant");
    }

    /// Follows `changed`, a change's redirect to the page showing what it made
    /// once, as `browser` with the sealed cookie the change set, and returns that
    /// page, which shows `made`. Its answer deletes the cookie, and `made` shows
    /// no more: not on the page again, as on a reload, nor with the cookie on
    /// another page, `elsewhere`.
    async fn shown_once(
        &self,
        browser: &Browser,
        changed: &Response,
        made: &str,
        elsewhere: &str,
    ) -> String {
        let page = redirect(changed).to_owned();
        let (sealed, _) = cookie(changed, BrowserCookie::DashboardReveal);
        let carrying = Browser {
            user_id: browser.user_id,
            session_id: browser.session_id,
            cookie: format!(
                "{}; {}={sealed}",
                browser.cookie,
                BrowserCookie::DashboardReveal
            ),
        };
        let shown = self.visit(&carrying, &page).await;
        assert_eq!(shown.status(), StatusCode::OK, "{page}");
        let (deleted, attributes) = cookie(&shown, BrowserCookie::DashboardReveal);
        assert_eq!(deleted, "");
        assert!(attributes.contains("Max-Age=0"), "{attributes}");
        let shown = html(shown).await;
        assert!(shown.contains(made), "{shown}");
        let reloaded = html(self.visit(browser, &page).await).await;
        assert!(!reloaded.contains(made), "{reloaded}");
        let misplaced = html(self.visit(&carrying, elsewhere).await).await;
        assert!(!misplaced.contains(made), "{misplaced}");
        shown
    }

    /// Visits the dashboard's `path` as `browser`.
    async fn visit(&self, browser: &Browser, path: &str) -> Response {
        browser
            .visit(&self.dashboard, &format!("{DASHBOARD}{path}"))
            .await
    }

    /// Follows `response`'s redirect to auth as `browser`.
    async fn to_auth(&self, browser: &Browser, response: &Response) -> Response {
        let target = location(response);
        assert_eq!(target.origin().ascii_serialization(), AUTH_ORIGIN);
        browser
            .visit(&self.auth.a, &target[Position::BeforePath..])
            .await
    }

    /// Opens the dashboard's `path` in a browser of its own, which goes through
    /// the handoff page as `auth`'s browser: the dashboard's browser, holding the
    /// state of its sign-in, the handoff it asked for, and auth's answer.
    async fn hand_off(&self, auth: &Browser, path: &str) -> (Browser, HandoffQuery, Response) {
        let started = self.visit(&Browser::signed_out(), path).await;
        assert_eq!(started.status(), StatusCode::SEE_OTHER);
        let handoff = location(&started);
        assert_eq!(handoff.path(), routes::page::ADMIN);
        assert_eq!(parameter(&handoff, "return_to"), path);
        let (state, _) = cookie(&started, BrowserCookie::DashboardState);
        let query = HandoffQuery {
            state: state.to_owned(),
            return_to: path.to_owned(),
        };
        let dashboard = Browser::signed_out().with_cookie(BrowserCookie::DashboardState, state);
        (dashboard, query, self.to_auth(auth, &started).await)
    }

    /// Signs `auth`'s browser in to the dashboard, which opens its `path`, with a
    /// passkey proof of the handoff long past, so a change still needs a
    /// confirmation: the dashboard's browser, holding its session.
    async fn sign_in(&self, auth: &Browser, path: &str) -> Browser {
        let long_ago = Utc::now() - chrono::Duration::hours(1);
        self.confirmed(auth, path, long_ago).await
    }

    /// Signs `auth`'s browser in to the dashboard, which opens its `path`, with a
    /// passkey proof of the handoff at `proved_at`, as the handoff page asks for
    /// one: the dashboard's browser, holding its session.
    async fn confirmed(&self, auth: &Browser, path: &str, proved_at: DateTime<Utc>) -> Browser {
        let (dashboard, query, asked) = self.hand_off(auth, path).await;
        assert_eq!(asked.status(), StatusCode::OK, "asks for a passkey");
        let proof = AdminProof::unverified(auth.session_id, proved_at);
        let code_url = self
            .sign_in_service()
            .hand_off(&auth.owner(&self.database).await, &query, proof)
            .await
            .expect("handed off");
        self.redeem(&dashboard, &code_url, path).await
    }

    /// The dashboard's sign-in, as auth's handoff page runs it.
    fn sign_in_service(&self) -> DashboardSignIn {
        DashboardSignIn::new(
            self.database.clone(),
            AUTH_ORIGIN.parse().expect("auth origin"),
            DASHBOARD.parse().expect("dashboard origin"),
        )
    }

    /// Redeems the handoff `code_url` in the dashboard's `browser`, which then goes
    /// on to `path`: the browser, holding the dashboard's session.
    async fn redeem(&self, browser: &Browser, code_url: &Url, path: &str) -> Browser {
        assert_eq!(code_url.origin().ascii_serialization(), DASHBOARD);
        assert_eq!(code_url.path(), routes::dashboard::SIGN_IN);
        let signed_in = browser.visit(&self.dashboard, code_url.as_str()).await;
        assert_eq!(redirect(&signed_in), path);
        let (session, _) = cookie(&signed_in, BrowserCookie::Dashboard);
        let (state, _) = cookie(&signed_in, BrowserCookie::DashboardState);
        assert_eq!(state, "", "the state is done with");
        Browser::signed_out().with_cookie(BrowserCookie::Dashboard, session)
    }
}

impl Harness {
    /// The actor of the newest event of `kind` that `found` picks, if any.
    async fn actor_of(&self, kind: &str, found: impl Fn(&DomainEvent) -> bool) -> Option<Actor> {
        let filter = EventFilter {
            kind: Some(kind.into()),
            ..EventFilter::default()
        };
        let events = self
            .database
            .events()
            .list(&filter, 100)
            .await
            .expect("events");
        events
            .into_iter()
            .find(|stored| stored.event.known().is_some_and(&found))
            .map(|stored| stored.actor)
    }
}

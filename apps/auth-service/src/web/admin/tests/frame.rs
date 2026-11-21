//! What every page shares: the crumb, the top bar, the sidebar, the status bar
//! and the page explaining a server error.

use super::*;

/// A page that fails internally says so on the dashboard's page, which shows the
/// trace ID its `X-Trace-Id` header carries, for a report to quote.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_server_error_shows_its_trace_id() {
    let _capture = Telemetry::capture("info");
    let dashboard = DashboardConfig {
        listener: SocketAddr::from(([127, 0, 0, 1], 0)),
        origin: DASHBOARD.parse().expect("dashboard origin"),
    };
    let config = AuthConfig::test(&AUTH_ORIGIN.parse().expect("auth origin"));
    // Without the security layer, which gives every page its request context, every
    // page fails internally.
    let database = Replicas::database().await;
    let keys = config.test_keys(database.clone()).await;
    let passkeys = config
        .dashboard_passkeys(database.clone(), &keys)
        .expect("passkeys");
    let pages = AdminApplication::new(database, &config, &dashboard, passkeys).routes();
    let pages = HttpTelemetry::default().wrap(pages);
    let response = Browser::signed_out()
        .visit(&pages, &format!("{DASHBOARD}/users"))
        .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let trace_id = response.headers()[HttpTelemetry::TRACE_ID_HEADER]
        .to_str()
        .expect("trace ID")
        .to_owned();
    assert!(
        html(response)
            .await
            .contains(&format!(r#"<code id="reference">{trace_id}</code>"#)),
        "the page shows {trace_id}"
    );
}

/// A page inside a tab has the tab in its crumb, linking its page: an account's
/// reads "admin / users / handle", and the account's history "admin / users /
/// handle / security history", the handle linking the account. The tab's own
/// page names it once. The menus' names are lowercase; the account's are not.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn the_crumb_leads_through_the_tab() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let account = format!("/users/{}", owner.user_id);
    let dashboard = harness.sign_in(&admin, &account).await;
    let page = html(harness.visit(&dashboard, &account).await).await;
    let users = r#"<a href="/users">users</a>"#;
    assert!(page.contains(users), "{page}");
    // The menus' names are lowercase in the crumb; the account's own are not.
    assert!(page.contains(r#"<a href="/">admin</a>"#), "{page}");
    assert!(page.contains("<b>browser-"), "{page}");
    let page = html(harness.visit(&dashboard, "/users").await).await;
    assert!(!page.contains(users), "{page}");
    assert!(page.contains("<b>users</b>"), "{page}");
    let page = html(
        harness
            .visit(&dashboard, &format!("{account}/history"))
            .await,
    )
    .await;
    assert!(
        page.contains(&format!(r#"<a href="{account}">browser-"#)),
        "{page}"
    );
    assert!(page.contains("<b>security history</b>"), "{page}");
}

/// The top bar's bell counts the administrator's own unread notifications and
/// opens their inbox on auth; beside it, a shortcut opens the Alerts tab.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn the_top_bar_has_the_administrators_bell_and_the_alerts() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let dashboard = harness.sign_in(&admin, "/users").await;
    let message = Notification::Message {
        title: "Hello".to_owned(),
        text: "From the team.".to_owned(),
    };
    let mut notifications = harness.database.notifications();
    notifications
        .insert(admin.user_id, None, &message)
        .await
        .expect("notify");
    // Raising an alert in another test notifies every administrator.
    let unread = notifications.unread(admin.user_id).await.expect("unread");
    assert!(unread >= 1);
    let page = html(harness.visit(&dashboard, "/users").await).await;
    let bell = format!(
        r#"{}" title="Notifications" aria-label="Notifications, {unread} unread">"#,
        routes::page::settings::NOTIFICATIONS
    );
    assert!(page.contains(&bell), "{page}");
    let alerts = format!(
        r#"<a class="shortcut alerts" href="{}" title="Alerts""#,
        routes::dashboard::ALERTS
    );
    assert!(page.contains(&alerts), "{page}");
}

/// An administrator's settings sidebar links the dashboard; nobody else's does.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn administrators_sidebars_link_the_dashboard() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let other = Browser::new_account(&harness.database).await;
    let page = html(admin.visit(&harness.auth.a, "/settings").await).await;
    assert!(page.contains("Admin dashboard"), "{page}");
    assert!(page.contains(DASHBOARD), "{page}");
    let page = html(other.visit(&harness.auth.a, "/settings").await).await;
    assert!(!page.contains("Admin dashboard"), "{page}");
}

/// The top bar's search holds the dashboard's pages and actions, and asks
/// `/search` for the accounts a query finds, each linking its page; an empty
/// query finds none.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn the_top_bar_finds_pages_and_accounts() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let other = Browser::new_account(&harness.database).await;
    let dashboard = harness.sign_in(&admin, "/users").await;
    let page = html(harness.visit(&dashboard, "/users").await).await;
    assert!(page.contains(r#"data-suggest="/search""#), "{page}");
    assert!(
        page.contains(
            r#"href="/keys/rotate" data-terms="new key jwks rotate" data-confirm-first="""#
        ),
        "an action, confirming first: {page}"
    );
    let search = format!("/search?q=%40{}", other.user_id);
    let found = html(harness.visit(&dashboard, &search).await).await;
    assert!(found.contains(r#"data-group="accounts""#), "{found}");
    assert!(
        found.contains(&format!(r#"href="/users/{}""#, other.user_id)),
        "{found}"
    );
    let none = html(harness.visit(&dashboard, "/search?q=+").await).await;
    assert!(!none.contains("data-group"), "{none}");
}

/// What every page shows keeps a read only if nothing forgot it meanwhile, so a
/// read under way while an administrator posts an update never brings back
/// what the update changed.
#[test]
fn a_read_started_before_forgetting_is_not_kept() {
    let mut cache = super::AttentionCache::default();
    let Err(started) = cache.fresh() else {
        panic!("nothing found yet");
    };
    cache.forget();
    cache.keep(started, super::Attention::default());
    let Err(now) = cache.fresh() else {
        panic!("the earlier read is dropped");
    };
    cache.keep(now, super::Attention::default());
    assert!(cache.fresh().is_ok(), "a read started since is kept");
}

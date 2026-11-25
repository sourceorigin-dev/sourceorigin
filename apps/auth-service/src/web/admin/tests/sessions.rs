//! Signing in to the dashboard through auth's handoff page, the dashboard's own
//! passkey confirmation, and the admin listener's security layer.

use super::*;

/// An administrator's passkey session signs in to the dashboard through auth's
/// handoff page, which asks for a passkey every time, and the sign-in is recorded
/// as theirs. The code works once, only in the browser holding the state it was
/// bound to. Every page loads the dashboard's script alone, and posts only to
/// itself.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn signing_in_asks_auth_for_a_passkey_once() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let (dashboard, query, asked) = harness.hand_off(&admin, "/users").await;
    assert_eq!(asked.status(), StatusCode::OK);
    assert!(
        html(asked)
            .await
            .contains(r#"data-passkey-assertion="/admin/proof""#),
        "asks for a passkey"
    );
    let proof = AdminProof::unverified(admin.session_id, Utc::now());
    let code_url = harness
        .sign_in_service()
        .hand_off(&admin.owner(&harness.database).await, &query, proof)
        .await
        .expect("handed off");
    let elsewhere = Browser::signed_out().with_cookie(BrowserCookie::DashboardState, "AAAA");
    let refused = elsewhere.visit(&harness.dashboard, code_url.as_str()).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "another browser");
    let signed_in = harness.redeem(&dashboard, &code_url, "/users").await;
    let replayed = dashboard.visit(&harness.dashboard, code_url.as_str()).await;
    assert_eq!(replayed.status(), StatusCode::BAD_REQUEST, "once only");
    let recorded = harness
        .actor_of("dashboard_signed_in", |event| {
            *event
                == DomainEvent::DashboardSignedIn {
                    session_id: admin.session_id,
                }
        })
        .await;
    assert!(
        matches!(recorded, Some(Actor::User { user_id, .. }) if user_id == admin.user_id),
        "{recorded:?}"
    );

    let page = harness.visit(&signed_in, "/users").await;
    assert_eq!(page.status(), StatusCode::OK);
    let csp = policy(&page).to_owned();
    let handoff = format!("; form-action 'self' {AUTH_ORIGIN}{}", routes::page::ADMIN);
    assert!(csp.ends_with(&handoff), "{csp}");
    let (script, search, copy, client_form) = (
        EmbeddedAssets::DASHBOARD_SCRIPT.path(),
        EmbeddedAssets::SEARCH_SCRIPT.path(),
        EmbeddedAssets::COPY_SCRIPT.path(),
        EmbeddedAssets::CLIENT_FORM_SCRIPT.path(),
    );
    assert!(
        csp.contains(&format!(
            "script-src {DASHBOARD}{script} {DASHBOARD}{search} {DASHBOARD}{copy} {DASHBOARD}{client_form};"
        )) && csp.contains("connect-src 'self';")
            && csp.contains(&format!("img-src {AUTH_ORIGIN};")),
        "{csp}"
    );
    let handle = format!("@browser-{}", &admin.user_id.to_string()[24..]);
    assert!(
        html(page).await.contains(&handle),
        "the bar names the administrator"
    );
}

/// A dashboard session ends with its auth session, and with the account's
/// rights: granting them again revives no old cookie.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn the_dashboard_ends_with_the_auth_session_and_the_rights() {
    let harness = Harness::new().await;
    let signed_out = harness.admin().await;
    let dashboard = harness.sign_in(&signed_out, "/users").await;
    let mut tx = harness.database.begin().await.expect("transaction");
    crate::tests::sign_out(&mut tx, signed_out.user_id, signed_out.session_id)
        .await
        .expect("sign out");
    tx.commit().await.expect("commit");
    let page = harness.visit(&dashboard, "/users").await;
    assert_eq!(
        location(&page).path(),
        routes::page::ADMIN,
        "signs in again"
    );

    let revoked = harness.admin().await;
    let dashboard = harness.sign_in(&revoked, "/users").await;
    harness.admin().await;
    AdminService::new(harness.database.clone(), true)
        .revoke_administrator(&operator(), revoked.user_id, reason())
        .await
        .expect("revoke");
    let ended = DomainEvent::DashboardSignedOut {
        session_id: revoked.session_id,
    };
    let recorded = harness
        .actor_of("dashboard_signed_out", |event| *event == ended)
        .await;
    assert!(matches!(recorded, Some(Actor::Cli { .. })), "{recorded:?}");
    harness.grant(revoked.user_id).await;
    let page = harness.visit(&dashboard, "/users").await;
    assert_eq!(
        location(&page).path(),
        routes::page::ADMIN,
        "granting the rights again revives no session"
    );
}

/// A sign-in and a revocation of the account's rights at once: whichever holds
/// the rights first, no handoff or dashboard session outlives the revocation.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_sign_in_racing_a_revocation_never_outlives_it() {
    let harness = Harness::new().await;
    harness.admin().await;
    let service = AdminService::new(harness.database.clone(), true);
    let database = &harness.database;
    let in_use = |user_id| async move {
        (database.dashboard())
            .of_user(user_id, DashboardSignIn::idle_since())
            .await
            .expect("sessions")
    };

    let admin = harness.admin().await;
    let mut redeeming = harness.database.begin().await.expect("transaction");
    let held = redeeming
        .administrators()
        .contains_for_share(admin.user_id)
        .await;
    assert!(held.expect("rights"));
    let cli = operator();
    let (revoked, ()) = tokio::join!(
        service.revoke_administrator(&cli, admin.user_id, reason()),
        async {
            redeeming.until_blocked(1).await;
            let expires_at = Utc::now() + chrono::Duration::hours(1);
            (redeeming.dashboard())
                .insert_session(b"in flight", admin.session_id, Utc::now(), expires_at)
                .await
                .expect("session");
            redeeming.commit().await.expect("commit");
        }
    );
    revoked.expect("revoke");
    assert!(
        in_use(admin.user_id).await.is_empty(),
        "ended by the revocation"
    );

    let admin = harness.admin().await;
    let (dashboard, query, _) = harness.hand_off(&admin, "/users").await;
    let owner = admin.owner(&harness.database).await;
    let sign_in = harness.sign_in_service();
    let proof = || AdminProof::unverified(admin.session_id, Utc::now());
    let code_url = sign_in
        .hand_off(&owner, &query, proof())
        .await
        .expect("handed off");
    let mut revoking = harness.database.begin().await.expect("transaction");
    let revoked = revoking.administrators().revoke(admin.user_id).await;
    assert!(revoked.expect("revoke"));
    let (handed, redeemed, ()) = tokio::join!(
        sign_in.hand_off(&owner, &query, proof()),
        dashboard.visit(&harness.dashboard, code_url.as_str()),
        async {
            revoking.until_blocked(2).await;
            revoking.commit().await.expect("commit");
        }
    );
    assert!(
        matches!(handed, Err(AdminError::NotAdministrator)),
        "{handed:?}"
    );
    assert_eq!(redeemed.status(), StatusCode::FORBIDDEN, "no access");
    harness.grant(admin.user_id).await;
    assert!(in_use(admin.user_id).await.is_empty(), "nothing revived");
}

/// A dashboard session times out once unused for half an hour: each request
/// keeps it going.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn an_idle_dashboard_session_expires() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let dashboard = harness.sign_in(&admin, "/users").await;
    let database = &harness.database;
    let idle = |minutes| async move {
        database
            .dashboard()
            .backdate_use(admin.session_id, chrono::Duration::minutes(minutes))
            .await
            .expect("backdate");
    };
    idle(29).await;
    let page = harness.visit(&dashboard, "/users").await;
    assert_eq!(page.status(), StatusCode::OK, "still in use");
    idle(29).await;
    let page = harness.visit(&dashboard, "/users").await;
    assert_eq!(page.status(), StatusCode::OK, "the last request counts");
    idle(31).await;
    let page = harness.visit(&dashboard, "/users").await;
    assert_eq!(location(&page).path(), routes::page::ADMIN, "timed out");
}

/// A change needs the dashboard's own passkey confirmation: a passkey sign-in
/// to auth moments ago does not count. The page asks auth's handoff page for
/// one, which asks for a passkey proof whose form may go on to the dashboard. A
/// proof of the session handing off confirms the dashboard; one of another
/// session does not.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_change_needs_a_confirmation_done_for_the_dashboard() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let user = Browser::new_account(&harness.database).await;
    let dashboard = harness.sign_in(&admin, "/users").await;
    let disable = format!("/users/{}/disable", user.user_id);
    let asked = harness.visit(&dashboard, &disable).await;
    let handoff = location(&asked);
    assert_eq!(handoff.path(), routes::page::ADMIN);
    let (state, _) = cookie(&asked, BrowserCookie::DashboardState);
    let page = harness.to_auth(&admin, &asked).await;
    assert_eq!(page.status(), StatusCode::OK);
    let csp = policy(&page).to_owned();
    assert!(
        csp.contains(&format!("form-action 'self' {DASHBOARD}")),
        "{csp}"
    );
    assert!(
        html(page)
            .await
            .contains(r#"data-passkey-assertion="/admin/proof""#)
    );

    let sign_in = harness.sign_in_service();
    let owner = admin.owner(&harness.database).await;
    let query = HandoffQuery {
        state: state.to_owned(),
        return_to: disable.clone(),
    };
    let other = Browser::sign_in(&harness.database, admin.user_id).await;
    let elsewhere = AdminProof::unverified(other.session_id, Utc::now());
    let refused = sign_in.hand_off(&owner, &query, elsewhere).await;
    assert!(refused.is_err(), "a proof of another session");
    let proof = AdminProof::unverified(admin.session_id, Utc::now());
    let code_url = sign_in
        .hand_off(&owner, &query, proof)
        .await
        .expect("handed off");
    let browser = dashboard.with_cookie(BrowserCookie::DashboardState, state);
    let confirmed = harness.redeem(&browser, &code_url, &disable).await;
    let form = harness.visit(&confirmed, &disable).await;
    assert_eq!(form.status(), StatusCode::OK);
    let done = confirmed
        .submit(
            &harness.dashboard,
            &format!("{DASHBOARD}{disable}"),
            &[("reason", "Browser test")],
            OWN_PAGE,
        )
        .await;
    assert_eq!(redirect(&done), format!("/users/{}", user.user_id));
    let stored = harness
        .database
        .users()
        .find_by_id(user.user_id)
        .await
        .expect("lookup")
        .expect("user");
    assert_eq!(stored.status, AccountStatus::Disabled);
}

/// The frame's dialog confirms the dashboard on its own page: it says how long
/// the confirmation lasts, nothing before it and five minutes after, and starts
/// the passkey ceremony for auth's RP ID, bound to the browser, as auth's own
/// pages do.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn the_dialog_confirms_on_the_page() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let dashboard = harness.sign_in(&admin, "/users").await;
    let page = html(harness.visit(&dashboard, "/users").await).await;
    assert!(
        page.contains(
            r#"data-confirmation-start="/confirm/start" data-confirmation-finish="/confirm/finish" data-confirmed-for="0" data-form-time="120""#
        ),
        "{page}"
    );
    let url = format!("{DASHBOARD}/confirm/start");
    let started = dashboard
        .submit(&harness.dashboard, &url, &[], OWN_PAGE)
        .await;
    assert_eq!(started.status(), StatusCode::OK);
    cookie(&started, BrowserCookie::Ceremony);
    let body: serde_json::Value = serde_json::from_str(&html(started).await).expect("JSON");
    assert!(body["ceremony_id"].is_string(), "{body}");
    let host = Url::parse(AUTH_ORIGIN).expect("auth origin");
    assert_eq!(
        body["public_key"]["publicKey"]["rpId"],
        host.host_str().expect("host")
    );

    let confirmed = harness.confirmed(&admin, "/users", Utc::now()).await;
    let page = html(harness.visit(&confirmed, "/users").await).await;
    assert!(!page.contains(r#"data-confirmed-for="0""#), "{page}");
}

/// Only an administrator signed in with a passkey is handed off: anyone else is
/// told they have no access, and a social sign-in signs in with a passkey first,
/// coming back to the handoff, which the sign-in page's forms may then follow to
/// the dashboard.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn only_administrators_signed_in_with_a_passkey_are_handed_off() {
    let harness = Harness::new().await;
    let user = Browser::new_account(&harness.database).await;
    let (_, _, refused) = harness.hand_off(&user, "/users").await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert!(refused.headers().get(header::LOCATION).is_none());
    assert!(html(refused).await.contains("This account has no access"));

    let social = Browser::new_account(&harness.database).await;
    harness.grant(social.user_id).await;
    let social = Browser::sign_in_with(
        &harness.database,
        social.user_id,
        AuthenticationMethod::Federation,
    )
    .await;
    let (_, _, asked) = harness.hand_off(&social, "/users").await;
    let login = Url::parse(AUTH_ORIGIN)
        .and_then(|auth| auth.join(redirect(&asked)))
        .expect("sign-in URL");
    assert_eq!(login.path(), routes::page::LOGIN);
    assert_eq!(parameter(&login, "acr"), "phr");
    let back = parameter(&login, "return_to");
    assert!(back.starts_with(routes::page::ADMIN), "{back}");
    let page = social
        .visit(&harness.auth.a, &login[Position::BeforePath..])
        .await;
    let csp = policy(&page).to_owned();
    assert!(
        csp.contains(&format!("form-action 'self' {DASHBOARD}")),
        "{csp}"
    );
    let elsewhere = format!("{}?acr=phr", routes::page::LOGIN);
    let page = social.visit(&harness.auth.a, &elsewhere).await;
    assert_eq!(page.status(), StatusCode::OK);
    assert!(!policy(&page).contains(DASHBOARD), "only on its way there");
}

/// A change's form is shown only while the dashboard's confirmation has time
/// left to fill it in, but may be posted as long as the confirmation lasts.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_form_shown_in_time_may_be_posted_until_the_confirmation_ends() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let user = Browser::new_account(&harness.database).await;
    let disable = format!("/users/{}/disable", user.user_id);
    let ninety_seconds_left = Utc::now() - chrono::Duration::seconds(210);
    let confirmed = harness
        .confirmed(&admin, &disable, ninety_seconds_left)
        .await;
    let form = harness.visit(&confirmed, &disable).await;
    assert_eq!(
        location(&form).path(),
        routes::page::ADMIN,
        "too late to show"
    );
    let done = confirmed
        .submit(
            &harness.dashboard,
            &format!("{DASHBOARD}{disable}"),
            &[("reason", "Posted in time")],
            OWN_PAGE,
        )
        .await;
    assert_eq!(redirect(&done), format!("/users/{}", user.user_id));
}

/// The admin listener serves only the dashboard's host, takes forms only from
/// its own pages, and signing out ends the dashboard's session alone, recorded
/// as the administrator's.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn the_admin_listener_keeps_to_its_host_forms_and_session() {
    let harness = Harness::new().await;
    let anyone = Browser::signed_out();
    let misrouted = anyone.visit(&harness.dashboard, "/users").await;
    assert_eq!(misrouted.status(), StatusCode::MISDIRECTED_REQUEST);
    let elsewhere = anyone
        .visit(&harness.dashboard, "https://auth.sourceorigin.dev/users")
        .await;
    assert_eq!(elsewhere.status(), StatusCode::MISDIRECTED_REQUEST);

    let admin = harness.admin().await;
    let dashboard = harness.sign_in(&admin, "/users").await;
    let sign_out = format!("{DASHBOARD}/sign-out");
    let foreign = [("sec-fetch-site", "same-site"), ("origin", AUTH_ORIGIN)];
    let refused = dashboard
        .submit(&harness.dashboard, &sign_out, &[], &foreign)
        .await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let signed_out = dashboard
        .submit(&harness.dashboard, &sign_out, &[], OWN_PAGE)
        .await;
    assert_eq!(redirect(&signed_out), "/sign-out");
    assert_eq!(cookie(&signed_out, BrowserCookie::Dashboard).0, "");
    let ended = DomainEvent::DashboardSignedOut {
        session_id: admin.session_id,
    };
    let recorded = harness
        .actor_of("dashboard_signed_out", |event| *event == ended)
        .await;
    assert!(
        matches!(recorded, Some(Actor::User { user_id, .. }) if user_id == admin.user_id),
        "recorded as the administrator's: {recorded:?}"
    );
    let page = harness.visit(&dashboard, "/users").await;
    assert_eq!(location(&page).path(), routes::page::ADMIN, "signed out");
    AccountService::new(harness.database.clone())
        .owner(RequestContext::TEST, &admin.headers())
        .await
        .expect("lookup")
        .expect("still signed in to Origin Auth");
}

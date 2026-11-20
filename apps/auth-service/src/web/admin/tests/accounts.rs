//! An account's page, the changes made to it and its settings, granting
//! rights, and messages to it.

use super::*;

/// An administrator removes the access of an app the account consented to, with
/// a reason, from its Connected apps page.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn administrators_remove_an_accounts_app() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let client = crate::tests::registration(
        sourceorigin_common::clients::ClientKind::Public,
        "https://client.example",
    );
    crate::tests::register(&harness.database, &client)
        .await
        .expect("register");
    let scopes = ["openid".to_owned()];
    harness
        .database
        .oidc()
        .save_consent(owner.user_id, &client.id, client.id.as_str(), &scopes)
        .await
        .expect("consent");
    let apps = format!("/users/{}/apps", owner.user_id);
    let dashboard = harness.confirmed(&admin, &apps, Utc::now()).await;
    let opened = format!("{apps}?remove={}", client.id);
    let page = html(harness.visit(&dashboard, &opened).await).await;
    assert!(page.contains("the owner’s approval again"), "{page}");
    assert!(
        page.contains(r#"name="reason""#),
        "a removal asks why: {page}"
    );

    let url = format!("{DASHBOARD}{apps}");
    let remove = [
        ("action", "remove"),
        ("client_id", client.id.as_str()),
        ("reason", "Reported as an impersonating app"),
    ];
    let done = dashboard
        .submit(&harness.dashboard, &url, &remove, OWN_PAGE)
        .await;
    assert_eq!(redirect(&done), apps);
    let page = html(harness.visit(&dashboard, &apps).await).await;
    assert!(
        page.contains("No app has access to this account."),
        "{page}"
    );
}

/// An administrator removes the picture of a disabled account, which shows
/// nowhere while the account is disabled, with a reason, from its Profile page.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn administrators_remove_a_disabled_accounts_picture() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    harness
        .database
        .pictures()
        .set(owner.user_id, PictureId::new(), b"picture")
        .await
        .expect("picture");
    harness
        .database
        .users()
        .set_status(owner.user_id, AccountStatus::Disabled)
        .await
        .expect("disable");
    let profile = format!("/users/{}/profile", owner.user_id);
    let dashboard = harness.confirmed(&admin, &profile, Utc::now()).await;
    let page = html(harness.visit(&dashboard, &profile).await).await;
    assert!(
        !page.contains(r#"<img class="avatar"#),
        "nothing to load: {page}"
    );
    assert!(page.contains("Remove picture"), "{page}");
    // Its changes on other pages ask for a passkey on this one first.
    let handle = format!("/users/{}/handle", owner.user_id);
    assert!(
        page.contains(&format!(
            r#"<a data-confirm-first="" class="button secondary small" href="{handle}""#
        )),
        "{page}"
    );

    let url = format!("{DASHBOARD}{profile}");
    let remove = [("action", "remove"), ("reason", "Impersonates someone")];
    let done = dashboard
        .submit(&harness.dashboard, &url, &remove, OWN_PAGE)
        .await;
    assert_eq!(redirect(&done), profile);
    let stored = harness.database.pictures().find(owner.user_id).await;
    assert!(stored.expect("read").is_none(), "removed");
}

/// A change the service refuses shows the form again with what was typed, at
/// the refusal's status: a settings section reopens the refused row with its
/// input and reason, which no other form takes, and a change page keeps its
/// fields.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_refused_change_keeps_what_was_typed() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let user = owner.user_id;
    let passkey = harness
        .database
        .passkeys()
        .insert(&crate::tests::unusable_passkey(user))
        .await
        .expect("passkey")
        .id
        .to_string();
    let sign_in = format!("/users/{user}/sign-in");
    let dashboard = harness.confirmed(&admin, &sign_in, Utc::now()).await;
    let post = async |path: &str, fields: &[(&str, &str)]| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, fields, OWN_PAGE)
            .await
    };

    let too_long = "x".repeat(81);
    let rename = [
        ("action", "rename"),
        ("id", passkey.as_str()),
        ("friendly_name", too_long.as_str()),
        ("reason", "Asked by email"),
    ];
    let refused = post(&sign_in, &rename).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let page = html(refused).await;
    assert!(page.contains(&format!(r#"value="{too_long}""#)), "{page}");
    assert!(page.contains(r#"value="Asked by email""#), "{page}");

    let first = format!("first-{user}@example.com");
    let primary = crate::tests::verified_address(&harness.database, user, &first, true).await;
    let second = format!("second-{user}@example.com");
    let second_id = crate::tests::verified_address(&harness.database, user, &second, false).await;
    let emails = format!("/users/{user}/emails");
    let primary = primary.to_string();
    let remove = [
        ("action", "remove"),
        ("id", primary.as_str()),
        ("reason", "Asked by phone"),
    ];
    let page = html(post(&emails, &remove).await).await;
    assert_eq!(
        page.matches(r#"value="Asked by phone""#).count(),
        1,
        "only the refused row keeps the reason: {page}"
    );
    let other = second_id.to_string();
    let long = "x".repeat(501);
    let primary = [
        ("action", "primary"),
        ("id", other.as_str()),
        ("reason", long.as_str()),
    ];
    let page = html(post(&emails, &primary).await).await;
    assert_eq!(
        page.matches(&format!(r#"value="{long}""#)).count(),
        1,
        "the refused Make primary keeps its own reason: {page}"
    );

    let handle = format!("/users/{user}/handle");
    let current = harness
        .database
        .users()
        .find_by_id(user)
        .await
        .expect("lookup")
        .expect("user")
        .handle
        .to_string();
    let same = [("handle", current.as_str()), ("reason", "Impersonation")];
    let refused = post(&handle, &same).await;
    assert!(
        refused.status().is_client_error(),
        "a change page keeps the refusal's status: {}",
        refused.status()
    );
    let page = html(refused).await;
    assert!(page.contains(&format!(r#"value="{current}""#)), "{page}");
    assert!(page.contains(r#"value="Impersonation""#), "{page}");
}

/// An account's page lists its passkeys by ID and when each was added, never by
/// name, which whoever used a recovery's link chose.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn an_accounts_page_names_no_passkey() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let passkey = harness
        .database
        .passkeys()
        .insert(&crate::tests::unusable_passkey(owner.user_id))
        .await
        .expect("passkey");
    let account = format!("/users/{}", owner.user_id);
    let dashboard = harness.sign_in(&admin, &account).await;
    let page = html(harness.visit(&dashboard, &account).await).await;
    assert!(page.contains(&passkey.id.to_string()), "{page}");
    assert!(!page.contains(&passkey.friendly_name), "{page}");
}

/// Recovering an account mails its primary verified address a link, which the
/// recovery pages then list, and undoing it cancels the unused link; both go on
/// to the account's page, whose events say which administrator started it and
/// why, as `audit list` does. Until then, the account's sign-in methods say no
/// passkey can be removed. An account whose link would not be mailed is left
/// alone, saying why.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn recoveries_are_started_and_undone_from_the_dashboard() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let address = format!("{}@example.com", owner.user_id);
    crate::tests::verified_address(&harness.database, owner.user_id, &address, true).await;
    let recover = format!("/users/{}/recover", owner.user_id);
    let dashboard = harness.confirmed(&admin, &recover, Utc::now()).await;
    let page = html(harness.visit(&dashboard, &recover).await).await;
    assert!(page.contains("No recovery in the last 15 days."));
    let reason = [("reason", "Lost every passkey")];
    let post = async |path: &str| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, &reason, OWN_PAGE)
            .await
    };
    let account = format!("/users/{}", owner.user_id);
    assert_eq!(redirect(&post(&recover).await), account);
    let page = html(harness.visit(&dashboard, &recover).await).await;
    assert!(page.contains("Link sent, not used"), "{page}");
    let handle = harness
        .database
        .users()
        .find_by_id(admin.user_id)
        .await
        .expect("lookup")
        .expect("administrator")
        .handle;
    let page = html(harness.visit(&dashboard, &account).await).await;
    // The recovery's row links its administrator by handle, then says why.
    let who_and_why = format!(
        r#"<a href="/users/{}">@{handle}</a>: “<span class="fact">Lost every passkey</span>”"#,
        admin.user_id
    );
    assert!(page.contains(&who_and_why), "{who_and_why}: {page}");
    let started = AdminService::new(harness.database.clone(), true)
        .events(
            &operator(),
            EventSearch {
                user: Some(owner.user_id),
                kind: Some("recovery_started".into()),
                ..EventSearch::default()
            },
        )
        .await
        .expect("events");
    let [started] = started.events.as_slice() else {
        panic!("one recovery started: {started:?}");
    };
    assert_eq!(started.administrator, Some(handle));
    assert_eq!(started.reason.as_deref(), Some("Lost every passkey"));
    assert!(!started.erased && started.ip.is_some(), "{started:?}");
    let sign_in = format!("/users/{}/sign-in", owner.user_id);
    let page = html(harness.visit(&dashboard, &sign_in).await).await;
    assert!(
        page.contains("removed while the recovery can be cancelled"),
        "{page}"
    );

    let undo = format!("/users/{}/undo-recovery", owner.user_id);
    assert_eq!(redirect(&post(&undo).await), account);
    let page = html(harness.visit(&dashboard, &undo).await).await;
    assert!(page.contains("Cancelled"), "{page}");

    let unreachable = Browser::new_account(&harness.database).await;
    let recover = format!("/users/{}/recover", unreachable.user_id);
    let refused = html(post(&recover).await).await;
    assert!(refused.contains("would not be mailed"), "{refused}");
    let recent = AdminService::new(harness.database.clone(), true)
        .recent_recoveries(&operator(), unreachable.user_id)
        .await
        .expect("recoveries");
    assert!(recent.is_empty(), "nothing started");
}

/// An administrator changes an account's settings with the owner's own
/// operations, each with a reason: renaming and removing a passkey, making
/// another address primary, and changing the display name from the account's
/// page. The pages offer nothing that adds a way in, and a change without a
/// reason, or only the owner's, is refused.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn administrators_change_an_accounts_settings_with_a_reason() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let user = owner.user_id;
    let mut passkeys = Vec::new();
    for _ in 0..2 {
        let stored = harness
            .database
            .passkeys()
            .insert(&crate::tests::unusable_passkey(user))
            .await
            .expect("passkey");
        passkeys.push(stored.id.to_string());
    }
    let first = format!("first-{user}@example.com");
    crate::tests::verified_address(&harness.database, user, &first, true).await;
    let second = format!("second-{user}@example.com");
    let other = crate::tests::verified_address(&harness.database, user, &second, false).await;
    let sign_in = format!("/users/{user}/sign-in");
    let dashboard = harness.confirmed(&admin, &sign_in, Utc::now()).await;
    let post = async |path: &str, fields: &[(&str, &str)]| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, fields, OWN_PAGE)
            .await
    };

    let page = html(harness.visit(&dashboard, &sign_in).await).await;
    assert!(page.contains("Existing key"), "{page}");
    assert!(!page.contains("data-passkey-enroll"), "no passkey is added");
    let opened = format!("{sign_in}?rename={}", passkeys[0]);
    let page = html(harness.visit(&dashboard, &opened).await).await;
    assert!(
        page.contains(r#"name="reason""#),
        "a change asks why: {page}"
    );

    let rename = [
        ("action", "rename"),
        ("id", passkeys[0].as_str()),
        ("friendly_name", "Renamed by support"),
    ];
    let refused = post(&sign_in, &[rename.as_slice(), &[("reason", "")]].concat()).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "no reason");
    let reason = ("reason", "Asked by email");
    let done = post(&sign_in, &[rename.as_slice(), &[reason]].concat()).await;
    assert_eq!(redirect(&done), sign_in);
    let remove = [("action", "remove"), ("id", passkeys[1].as_str()), reason];
    assert_eq!(redirect(&post(&sign_in, &remove).await), sign_in);
    let active = harness
        .database
        .passkeys()
        .list_active(user)
        .await
        .expect("passkeys");
    let names: Vec<_> = active
        .iter()
        .map(|key| key.friendly_name.as_str())
        .collect();
    assert_eq!(names, ["Renamed by support"]);
    let link = [("action", "link"), ("provider", "github"), reason];
    let refused = html(post(&sign_in, &link).await).await;
    assert!(
        refused.contains("Only the account&#39;s owner")
            || refused.contains("Only the account's owner")
    );

    let emails = format!("/users/{user}/emails");
    let page = html(harness.visit(&dashboard, &emails).await).await;
    assert!(!page.contains("Add an address"), "no address is added");
    let other = other.to_string();
    let primary = [("action", "primary"), ("id", other.as_str()), reason];
    assert_eq!(redirect(&post(&emails, &primary).await), emails);
    let stored = harness.database.emails().list(user).await.expect("emails");
    let primary = stored
        .iter()
        .find(|email| email.is_primary)
        .expect("primary");
    assert_eq!(primary.id.to_string(), other);

    let account = format!("/users/{user}");
    let name = format!("{account}/name");
    let page = html(harness.visit(&dashboard, &account).await).await;
    assert!(
        page.contains(&format!(r#"href="{name}">Change display name</a>"#)),
        "{page}"
    );
    // A refused name keeps the reason typed beside it.
    let page = html(post(&name, &[("display_name", " "), reason]).await).await;
    assert!(page.contains("Choose a display name"), "{page}");
    assert!(page.contains(r#"value="Asked by email""#), "{page}");
    let done = post(&name, &[("display_name", " Renamed by support "), reason]).await;
    assert_eq!(redirect(&done), format!("{account}/profile"));
    let stored = harness
        .database
        .users()
        .find_by_id(user)
        .await
        .expect("lookup")
        .expect("user");
    assert_eq!(stored.display_name, "Renamed by support");
}

/// Granting rights starts from a search: the page lists the accounts matching a
/// handle, with a button for each that may be granted, and the button's form
/// makes the account an administrator.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn rights_are_granted_from_a_search() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let other = Browser::new_account(&harness.database).await;
    let dashboard = harness
        .confirmed(&admin, "/administrators/grant", Utc::now())
        .await;
    let empty = html(harness.visit(&dashboard, "/administrators/grant").await).await;
    assert!(empty.contains("Search for the account"), "{empty}");
    assert!(
        !empty.contains(r#"name="user_id""#),
        "nothing listed: {empty}"
    );
    let search = format!("/administrators/grant?q={}", other.user_id);
    let page = html(harness.visit(&dashboard, &search).await).await;
    assert!(
        page.contains(&format!(r#"name="user_id" value="{}""#, other.user_id)),
        "{page}"
    );
    let granted = dashboard
        .submit(
            &harness.dashboard,
            &format!("{DASHBOARD}/administrators/grant"),
            &[("user_id", other.user_id.to_string().as_str()), REASON],
            OWN_PAGE,
        )
        .await;
    assert_eq!(redirect(&granted), "/administrators");
    let page = html(harness.visit(&dashboard, &search).await).await;
    assert!(page.contains(">Administrator<"), "{page}");
    assert!(
        !page.contains(r#"name="user_id""#),
        "granted already: {page}"
    );
}

/// An administrator schedules an account's deletion and restores it from the
/// dashboard, each with a reason; the account's page then offers Restore, not
/// Enable, and only the terminal purges at once.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn deletions_are_scheduled_and_restored_from_the_dashboard() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let account = format!("/users/{}", owner.user_id);
    let delete = format!("{account}/delete");
    let dashboard = harness.confirmed(&admin, &delete, Utc::now()).await;
    // Confirmed, the dashboard shows the change's form on the account's page.
    let page = html(harness.visit(&dashboard, &account).await).await;
    assert!(page.contains(&format!(r#"action="{delete}""#)), "{page}");
    let page = html(harness.visit(&dashboard, &delete).await).await;
    assert!(
        page.contains("--immediately"),
        "the terminal purges: {page}"
    );
    let reason = [("reason", "Asked by email")];
    let post = async |path: &str| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, &reason, OWN_PAGE)
            .await
    };
    assert_eq!(redirect(&post(&delete).await), account);
    let status = async || {
        harness
            .database
            .users()
            .find_by_id(owner.user_id)
            .await
            .expect("lookup")
            .expect("account")
            .status
    };
    assert_eq!(status().await, AccountStatus::Deleting);

    let restore = format!("{account}/restore");
    let page = html(harness.visit(&dashboard, &account).await).await;
    assert!(page.contains(&format!(r#"action="{restore}""#)), "{page}");
    assert!(page.contains(">Restore<"), "{page}");
    assert!(!page.contains("/enable"), "{page}");
    assert_eq!(redirect(&post(&restore).await), account);
    assert_eq!(status().await, AccountStatus::Active);
}

/// An administrator reads an account's inbox and security history as its owner
/// sees them, from the account's settings tabs, without a confirmation: each
/// notification's text in full, marking nothing read, and each change with who
/// made it and why, reporting nothing.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn administrators_read_an_accounts_inbox_and_history() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let renamed =
        Handle::parse(&format!("told-{}", &owner.user_id.to_string()[24..])).expect("handle");
    AdminService::new(harness.database.clone(), true)
        .rename_handle(
            &operator(),
            owner.user_id,
            renamed,
            Reason::parse("Asked by email".into()).expect("reason"),
            false,
        )
        .await
        .expect("rename");
    let inbox = format!("/users/{}/notifications", owner.user_id);
    let history = format!("/users/{}/history", owner.user_id);
    let dashboard = harness.sign_in(&admin, &inbox).await;

    let page = html(harness.visit(&dashboard, &inbox).await).await;
    assert!(
        page.contains("SourceOrigin support changed your handle"),
        "{page}"
    );
    assert!(
        page.contains("You now sign in with your new handle"),
        "{page}"
    );
    assert!(page.contains("Not read yet"), "{page}");
    assert!(
        !page.contains("Mark all as read"),
        "only the owner marks: {page}"
    );
    let unread = harness
        .database
        .notifications()
        .unread(owner.user_id)
        .await
        .expect("unread");
    assert_eq!(unread, 1);

    let page = html(harness.visit(&dashboard, &history).await).await;
    assert!(page.contains("as its owner sees it"), "{page}");
    assert!(page.contains("Handle changed"), "{page}");
    assert!(
        page.contains("By a SourceOrigin operator: “Asked by email”"),
        "{page}"
    );
    assert!(
        !page.contains("This wasn’t me"),
        "only the owner reports: {page}"
    );
    let sign_in = format!("/users/{}/sign-in", owner.user_id);
    let dashboard = harness.confirmed(&admin, &sign_in, Utc::now()).await;
    let page = html(harness.visit(&dashboard, &sign_in).await).await;
    assert!(
        page.contains(&format!(r#"href="{history}""#)),
        "a tab: {page}"
    );
}

/// A change's page names the account it changes, by handle and ID, beside its
/// button, and its form names it to the passkey dialog: two such pages left open
/// cannot be told apart only by their address.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_change_page_names_its_target() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let user = Browser::new_account(&harness.database).await;
    let disable = format!("/users/{}/disable", user.user_id);
    let dashboard = harness.confirmed(&admin, &disable, Utc::now()).await;
    let page = html(harness.visit(&dashboard, &disable).await).await;
    let handle = format!("browser-{}", &user.user_id.to_string()[24..]);
    assert!(
        page.contains(&format!(
            r#"<p class="target"><strong>{handle}</strong> <code>{}</code></p>"#,
            user.user_id
        )),
        "{page}"
    );
    assert!(
        page.contains(&format!(
            r#"data-confirming="Disable this account: {handle} ({})""#,
            user.user_id
        )),
        "{page}"
    );
}

/// An account's page revokes only its own sessions: another account's session
/// in the path is not found, and stays signed in.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_session_is_revoked_only_from_its_own_account() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let (named, other) = (
        Browser::new_account(&harness.database).await,
        Browser::new_account(&harness.database).await,
    );
    let revoke = format!(
        "/users/{}/sessions/{}/revoke",
        named.user_id, other.session_id
    );
    let dashboard = harness.confirmed(&admin, &revoke, Utc::now()).await;
    let refused = dashboard
        .submit(
            &harness.dashboard,
            &format!("{DASHBOARD}{revoke}"),
            &[REASON],
            OWN_PAGE,
        )
        .await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    let session = harness
        .database
        .sessions()
        .find_active_by_id(other.session_id)
        .await
        .expect("lookup");
    assert!(session.is_some(), "still signed in");
}

/// An account's page signs one of its sessions out, then every one, each with a
/// reason, and goes back to the account's page.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn sessions_are_revoked_from_the_dashboard() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    Browser::sign_in(&harness.database, owner.user_id).await;
    let account = format!("/users/{}", owner.user_id);
    let one = format!("{account}/sessions/{}/revoke", owner.session_id);
    let all = format!("{account}/sessions/revoke");
    let dashboard = harness.confirmed(&admin, &one, Utc::now()).await;
    let post = async |path: &str| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, &[REASON], OWN_PAGE)
            .await
    };
    let active = async || {
        harness
            .database
            .sessions()
            .list_active_for_user(owner.user_id)
            .await
            .expect("sessions")
            .len()
    };
    assert_eq!(active().await, 2);
    assert_eq!(redirect(&post(&one).await), account);
    assert_eq!(active().await, 1);
    assert_eq!(redirect(&post(&all).await), account);
    assert_eq!(active().await, 0);
}

/// An administrator writes a message from an account's page and sees its email
/// as it will be sent, both bodies, the HTML one drawn by the dashboard's
/// stylesheet, which its policy keeps from inline styles, without a fresh
/// confirmation. Sending it needs one, then
/// goes back to the account's page, and its owner reads it in their inbox. An
/// administrator's account takes one too.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn messages_are_previewed_then_sent_from_the_dashboard() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let account = format!("/users/{}", owner.user_id);
    let message = format!("{account}/message");
    let unconfirmed = harness.sign_in(&admin, &account).await;
    let page = html(harness.visit(&unconfirmed, &account).await).await;
    assert!(page.contains(&format!(r#"href="{message}""#)), "{page}");
    let typed = |intent| {
        [
            ("subject", "About your account"),
            (
                "body",
                "Support needs a word.\r\n\r\nWrite to support@example.com.",
            ),
            ("intent", intent),
        ]
    };
    let post = async |browser: &Browser, intent| {
        let url = format!("{DASHBOARD}{message}");
        browser
            .submit(&harness.dashboard, &url, &typed(intent), OWN_PAGE)
            .await
    };
    let preview = html(post(&unconfirmed, "preview").await).await;
    assert!(
        preview.contains("Hello,\n\nSupport needs a word.\n\nWrite to support@example.com."),
        "{preview}"
    );
    assert!(
        preview.contains(
            r#"<div class="email-preview"><p>Hello,</p><p>Support needs a word.</p><p>Write to support@example.com.</p><p class="email-fine">"#
        ),
        "{preview}"
    );
    assert!(!preview.contains("style="), "{preview}");
    assert!(preview.contains(r#"value="send""#), "{preview}");
    // Its Send names the account, beside it and to the passkey dialog.
    let handle = format!("browser-{}", &owner.user_id.to_string()[24..]);
    let target = format!(
        r#"<p class="target"><strong>{handle}</strong> <code>{}</code></p>"#,
        owner.user_id
    );
    assert!(preview.contains(&target), "{preview}");
    let confirming = format!(
        r#"data-confirming="Send a message: {handle} ({})""#,
        owner.user_id
    );
    assert!(preview.contains(&confirming), "{preview}");
    let inbox = async || {
        let mut notifications = harness.database.notifications();
        let received = notifications.list(owner.user_id, None, 10).await;
        received.expect("inbox")
    };
    // Sending without a confirmation confirms on auth's handoff page first.
    let unconfirmed_send = post(&unconfirmed, "send").await;
    assert_eq!(location(&unconfirmed_send).path(), routes::page::ADMIN);
    assert_eq!(inbox().await.len(), 0);

    let confirmed = harness.confirmed(&admin, &message, Utc::now()).await;
    assert_eq!(redirect(&post(&confirmed, "send").await), account);
    let [received] = <[_; 1]>::try_from(inbox().await).expect("one notification");
    let item = routes::page::settings::NOTIFICATION.replace("{id}", &received.id.to_string());
    let shown = html(owner.visit(&harness.auth.a, &item).await).await;
    assert!(shown.contains("About your account"), "{shown}");
    assert!(shown.contains("<p>Support needs a word.</p>"), "{shown}");

    // An administrator's account takes one too: it changes nothing.
    let administrator = format!("/users/{}", admin.user_id);
    let page = html(harness.visit(&confirmed, &administrator).await).await;
    assert!(
        page.contains(&format!(r#"href="{administrator}/message""#)),
        "{page}"
    );
    let form = harness
        .visit(&confirmed, &format!("{administrator}/message"))
        .await;
    assert_eq!(form.status(), StatusCode::OK);
    let to_administrator = format!("{DASHBOARD}{administrator}/message");
    let sent = confirmed
        .submit(
            &harness.dashboard,
            &to_administrator,
            &typed("send"),
            OWN_PAGE,
        )
        .await;
    assert_eq!(redirect(&sent), administrator);
    let mut notifications = harness.database.notifications();
    let received = notifications.list(admin.user_id, None, 10).await;
    let messages = received
        .expect("inbox")
        .into_iter()
        .filter(|stored| matches!(stored.notification, Some(Notification::Message { .. })))
        .count();
    assert_eq!(messages, 1, "the administrator reads it in their inbox");
}

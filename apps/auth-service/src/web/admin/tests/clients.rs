//! The OIDC clients: registering, changing, suspending and deleting them, and
//! the origins approved for their pages.

use super::*;

/// A client's page lists its approved origins. An administrator
/// approves one of a trusted client's redirect origins from its page, and
/// removes it again; a malformed origin, or one that is not a redirect origin, is
/// refused beside the form. An untrusted client's page says why it has none.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn related_origins_are_approved_and_removed_from_the_dashboard() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let origin = format!("https://{}.auth.sourceorigin.dev", UserId::new());
    let client = sourceorigin_common::clients::ClientRegistration {
        trusted: true,
        ..crate::tests::registration(sourceorigin_common::clients::ClientKind::Public, &origin)
    };
    crate::tests::register(&harness.database, &client)
        .await
        .expect("client");
    let untrusted =
        crate::tests::registration(sourceorigin_common::clients::ClientKind::Public, &origin);
    crate::tests::register(&harness.database, &untrusted)
        .await
        .expect("untrusted client");
    let approve = format!("/clients/{}/origins/approve", client.id);
    let dashboard = harness.confirmed(&admin, &approve, Utc::now()).await;
    let client_page = format!("/clients/{}/overview", client.id);
    let page = html(harness.visit(&dashboard, &client_page).await).await;
    assert!(page.contains(&approve), "{page}");
    let other = html(
        harness
            .visit(&dashboard, &format!("/clients/{}/overview", untrusted.id))
            .await,
    )
    .await;
    assert!(other.contains("pages sign users in themselves."), "{other}");
    let post = async |path: &str, fields: &[(&str, &str)]| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, fields, OWN_PAGE)
            .await
    };

    let refused = html(post(&approve, &[("origin", "notes.example"), REASON]).await).await;
    assert!(refused.contains("role=\"alert\""), "{refused}");
    let elsewhere = format!("https://elsewhere-{}.auth.sourceorigin.dev", UserId::new());
    let refused = html(post(&approve, &[("origin", &elsewhere), REASON]).await).await;
    assert!(
        refused.contains("is not the origin of any redirect URI"),
        "{refused}"
    );
    assert!(
        refused.contains(&format!("value=\"{elsewhere}\"")),
        "the refused origin is kept: {refused}"
    );

    assert_eq!(
        redirect(&post(&approve, &[("origin", &origin), REASON]).await),
        client_page
    );
    let page = html(harness.visit(&dashboard, &client_page).await).await;
    let remove = format!(
        "/clients/{}/origins/remove?origin={}",
        client.id,
        url::form_urlencoded::byte_serialize(origin.as_bytes()).collect::<String>()
    );
    assert!(page.contains(&remove), "listed: {page}");
    assert_eq!(redirect(&post(&remove, &[REASON]).await), client_page);
    let listed = AdminService::new(harness.database.clone(), true)
        .list_related_origins(&operator())
        .await
        .expect("origins");
    assert!(
        !listed
            .iter()
            .any(|approved| approved.origin.as_str() == origin)
    );
}

/// The stored registration of client `id`.
async fn stored_client(database: &Database, id: &str) -> sourceorigin_repository::StoredOidcClient {
    database
        .oidc()
        .find_client(id)
        .await
        .expect("lookup")
        .expect("client")
}

/// An administrator registers a confidential client with the settings only
/// operators choose, its secret shown once, on the page registering goes on
/// to; a refused registration keeps what was
/// typed. Its page then changes them, keeping the scopes beyond the standard ones,
/// suspends it, rotates its secret and deletes it.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn clients_are_registered_changed_and_suspended_from_the_dashboard() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let dashboard = harness.confirmed(&admin, "/clients/new", Utc::now()).await;
    let post = async |path: &str, fields: &[(&str, &str)]| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, fields, OWN_PAGE)
            .await
    };
    let id = format!("ops-{}", UserId::new());
    let registration = |redirect: &'static str| {
        [
            ("id", id.as_str()),
            REASON,
            ("display_name", "Ops app"),
            ("kind", "confidential"),
            ("redirect_uris", redirect),
            ("openid", "on"),
            ("other_scopes", "notes.read\r\n"),
            ("trusted", "on"),
            ("backchannel_logout_uri", "https://ops.example/backchannel"),
        ]
    };

    let refused = html(post("/clients/new", &registration("not a URI")).await).await;
    assert!(refused.contains("redirect_uris"), "{refused}");
    assert!(refused.contains(&format!(r#"value="{id}""#)), "{refused}");
    let created = post("/clients/new", &registration("https://ops.example/cb")).await;
    let client = format!("/clients/{id}");
    let page = format!("{client}/overview");
    assert_eq!(redirect(&created), page);
    let settings = format!("{client}/settings");
    harness
        .shown_once(&dashboard, &created, "socs_", &format!("{client}/secret"))
        .await;
    let stored = stored_client(&harness.database, &id).await;
    assert!(stored.trusted);
    assert_eq!(stored.owner_id, None);
    assert_eq!(stored.allowed_scopes, ["openid", "notes.read"]);
    assert_eq!(
        stored.backchannel_logout_uri.as_deref(),
        Some("https://ops.example/backchannel")
    );

    let list = html(harness.visit(&dashboard, "/clients").await).await;
    assert!(list.contains(&format!(r#"href="{page}""#)), "{list}");
    let form = html(harness.visit(&dashboard, &settings).await).await;
    assert!(form.contains(">notes.read</textarea>"), "{form}");
    let saved = post(
        &settings,
        &[
            ("display_name", "Ops app"),
            ("redirect_uris", "https://ops.example/cb"),
            ("openid", "on"),
            ("other_scopes", "notes.read"),
            ("backchannel_logout_uri", ""),
            REASON,
        ],
    )
    .await;
    assert_eq!(redirect(&saved), page);
    let stored = stored_client(&harness.database, &id).await;
    assert!(!stored.trusted, "an unchecked box clears the trust");
    assert_eq!(stored.backchannel_logout_uri, None);
    assert_eq!(stored.allowed_scopes, ["openid", "notes.read"]);

    assert_eq!(
        redirect(&post(&format!("{client}/suspend"), &[REASON]).await),
        page
    );
    assert!(
        stored_client(&harness.database, &id)
            .await
            .suspended_at
            .is_some()
    );
    let shown = html(harness.visit(&dashboard, &page).await).await;
    assert!(shown.contains("Lift the suspension first."), "{shown}");
    let enable = html(post(&format!("{client}/enable"), &[REASON]).await).await;
    assert!(enable.contains("lift the suspension"), "{enable}");

    let rotated = post(&format!("{client}/secret"), &[("action", "rotate"), REASON]).await;
    assert_eq!(redirect(&rotated), format!("{client}/secret"));
    harness
        .shown_once(&dashboard, &rotated, "socs_", &page)
        .await;
    let secrets = harness
        .database
        .oidc()
        .client_secrets(&id.parse().expect("client ID"))
        .await
        .expect("secrets");
    assert_eq!(secrets.len(), 2);
    assert!(secrets[0].expires_at.is_some(), "the first one is expiring");

    assert_eq!(
        redirect(&post(&format!("{client}/delete"), &[REASON]).await),
        "/clients"
    );
    let shown = html(harness.visit(&dashboard, &page).await).await;
    assert!(shown.contains("Deleted on"), "{shown}");
    assert!(
        !shown.contains(&settings),
        "a deleted client changes no more: {shown}"
    );
}

/// A page whose change the registry could only refuse says so in place of its
/// form: a public client's secrets page, and every change page of a deleted
/// client. Registering a taken ID says it is taken.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn pages_that_could_only_fail_refuse_at_once() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let dashboard = harness.confirmed(&admin, "/clients/new", Utc::now()).await;
    let post = async |path: &str, fields: &[(&str, &str)]| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, fields, OWN_PAGE)
            .await
    };
    let refused = async |path: &str, status: StatusCode, says: &str| {
        let response = harness.visit(&dashboard, path).await;
        assert_eq!(response.status(), status, "{path}");
        let page = html(response).await;
        assert!(page.contains(says), "{path}: {page}");
    };
    let id = format!("spa-{}", UserId::new());
    let registration = [
        ("id", id.as_str()),
        REASON,
        ("display_name", "Single-page app"),
        ("kind", "public"),
        ("redirect_uris", "https://spa.example/cb"),
        ("openid", "on"),
    ];
    let client = format!("/clients/{id}");
    let page = format!("{client}/overview");
    assert_eq!(redirect(&post("/clients/new", &registration).await), page);
    let taken = html(post("/clients/new", &registration).await).await;
    assert!(taken.contains("is taken"), "{taken}");

    let secret = format!("{client}/secret");
    refused(&secret, StatusCode::BAD_REQUEST, "is public").await;
    assert_eq!(
        redirect(&post(&format!("{client}/delete"), &[REASON]).await),
        "/clients"
    );
    for change in [
        "origins/approve",
        "settings",
        "enable",
        "disable",
        "suspend",
        "unsuspend",
        "delete",
        "secret",
    ] {
        refused(
            &format!("{client}/{change}"),
            StatusCode::CONFLICT,
            "was deleted",
        )
        .await;
    }
}

/// A client whose ID is `new` has its overview like any other, apart from the
/// registration page.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_client_named_new_has_its_overview() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let dashboard = harness.confirmed(&admin, "/clients/new", Utc::now()).await;
    if harness
        .database
        .oidc()
        .find_client("new")
        .await
        .expect("lookup")
        .is_none()
    {
        let client = sourceorigin_common::clients::ClientRegistration {
            id: "new".parse().expect("client ID"),
            ..crate::tests::registration(
                sourceorigin_common::clients::ClientKind::Public,
                "https://new.example",
            )
        };
        crate::tests::register(&harness.database, &client)
            .await
            .expect("client");
    }

    let overview = html(harness.visit(&dashboard, "/clients/new/overview").await).await;
    assert!(
        overview.contains(r#"href="/clients/new/settings""#),
        "{overview}"
    );
    let registration = html(harness.visit(&dashboard, "/clients/new").await).await;
    assert!(registration.contains(r#"name="id""#), "{registration}");
}

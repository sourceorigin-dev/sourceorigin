//! The APIs registered as resources: registering one, defining its scopes,
//! granting and revoking a client's access, and deleting it.

use super::*;

/// An administrator registers an API, and a refused registration keeps what was
/// typed. Its page then defines a scope, lets a client request it and renames
/// the API, lists both with the links that remove them, which do, and deletes
/// it.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn resources_are_registered_changed_and_deleted_from_the_dashboard() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let dashboard = harness
        .confirmed(&admin, "/resources/new", Utc::now())
        .await;
    let post = async |path: &str, fields: &[(&str, &str)]| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, fields, OWN_PAGE)
            .await
    };
    let identifier = format!("https://{}.example/api", UserId::new());
    let registration = |identifier: &'static str| {
        [
            ("identifier", identifier),
            ("display_name", "Notes API"),
            REASON,
        ]
    };
    let refused = html(post("/resources/new", &registration("notes")).await).await;
    assert!(refused.contains("is not an absolute URL"), "{refused}");
    assert!(refused.contains(r#"value="notes""#), "{refused}");
    let fields = [
        ("identifier", identifier.as_str()),
        ("display_name", "Notes API"),
        REASON,
    ];
    assert_eq!(
        redirect(&post("/resources/new", &fields).await),
        "/resources"
    );

    let service = AdminService::new(harness.database.clone(), true);
    let found = service
        .find_resource(&operator(), &identifier.parse().expect("identifier"))
        .await
        .expect("resource");
    assert_eq!(found.owner_id, None);
    let page = format!("/resources/{}", found.id);
    let list = html(harness.visit(&dashboard, "/resources").await).await;
    assert!(list.contains(&format!(r#"href="{page}""#)), "{list}");
    let client = crate::tests::registration(
        sourceorigin_common::clients::ClientKind::Public,
        "https://notes.example",
    );
    crate::tests::register(&harness.database, &client)
        .await
        .expect("client");
    for (path, fields) in [
        (
            "scopes/define",
            [("name", "notes:read"), ("description", "Read your notes")],
        ),
        (
            "access/grant",
            [("client", client.id.as_str()), ("scopes", "notes:read")],
        ),
        ("rename", [("display_name", "Notes"), ("unused", "")]),
    ] {
        let fields = [fields[0], fields[1], REASON];
        let saved = post(&format!("{page}/{path}"), &fields).await;
        assert_eq!(redirect(&saved), page, "{path}");
    }

    let shown = html(harness.visit(&dashboard, &page).await).await;
    let revoke = format!("{page}/access/revoke?client={}", client.id);
    let remove = format!("{page}/scopes/remove?scope=notes%3Aread");
    for expected in ["Read your notes", revoke.as_str(), remove.as_str()] {
        assert!(shown.contains(expected), "{expected}: {shown}");
    }
    for path in [&revoke, &remove] {
        assert_eq!(redirect(&post(path, &[REASON]).await), page, "{path}");
    }
    let detail = service
        .resource_detail(&operator(), found.id)
        .await
        .expect("detail");
    assert_eq!(detail.resource.display_name, "Notes");
    assert!(detail.scopes.is_empty() && detail.access.is_empty());

    let deleted = post(&format!("{page}/delete"), &[REASON]).await;
    assert_eq!(redirect(&deleted), "/resources");
    let list = html(harness.visit(&dashboard, "/resources").await).await;
    assert!(!list.contains(&identifier), "{list}");
}

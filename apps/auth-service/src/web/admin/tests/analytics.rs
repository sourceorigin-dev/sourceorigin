//! The Analytics page: what it charts, and what it says without a reader.

use super::*;
use chrono::{Duration, Utc};
use sourceorigin_common::EventId;

/// Without a reader for the analytics database, the page says analytics are
/// off, and every other page still works.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn the_analytics_page_says_when_analytics_are_off() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let dashboard = harness.sign_in(&admin, "/analytics").await;
    let page = html(harness.visit(&dashboard, "/analytics").await).await;
    assert!(page.contains("Analytics are off"), "{page}");
}

/// When ClickHouse refuses the reader, the page says the database didn't
/// answer, and the rest of the dashboard works.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and TEST_ANALYTICS_URL: run `just db test`"]
async fn the_analytics_page_says_when_clickhouse_refuses() {
    let stranger = Store::test().as_user("nobody_provisioned", "no-password");
    let config = Harness::reading(
        AuthConfig::test(&AUTH_ORIGIN.parse().expect("auth origin")),
        Some(stranger),
    );
    let harness = Harness::of(config).await;
    let admin = harness.admin().await;
    let dashboard = harness.sign_in(&admin, "/analytics").await;
    let page = html(harness.visit(&dashboard, "/analytics").await).await;
    assert!(page.contains("didn't answer"), "{page}");
    let overview = harness.visit(&dashboard, "/").await;
    assert_eq!(overview.status(), StatusCode::OK);
}

/// The page charts what ClickHouse counts, read as a reader `migrate`
/// provisioned, which may only read, every day of its sign-ins chart, no
/// request that matched no route, and every table's size, `logs` included.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and TEST_ANALYTICS_URL: run `just db test`"]
async fn the_dashboard_charts_what_clickhouse_counts() {
    let owner = Store::test();
    let reader = owner.as_user("dashboard_reader", "reader-password");
    owner
        .migrate(crate::cli::ANALYTICS_TABLES, None, Some(&reader))
        .await
        .expect("migrated");
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let seed = async |table: &'static str, row: serde_json::Value| {
        let mut row = row;
        row["time"] = now.to_string().into();
        owner
            .insert_rows(table, row.to_string())
            .await
            .expect("seeded");
    };
    seed(
        "requests",
        serde_json::json!({"service": "auth-service", "instance": "test", "listener": null,
            "method": "GET", "route": "/charted", "status": 200, "duration_us": 1500, "trace": null}),
    )
    .await;
    seed(
        "requests",
        serde_json::json!({"service": "auth-service", "instance": "test", "listener": null,
            "method": "GET", "route": null, "status": 404, "duration_us": 9_000_000, "trace": null}),
    )
    .await;
    seed(
        "logs",
        serde_json::json!({"trace": "0".repeat(32), "span": "0".repeat(16), "service": "auth-service",
            "instance": "test", "level": "INFO", "target": "test", "body": "stored", "attributes": {}}),
    )
    .await;
    seed(
        "events",
        serde_json::json!({"kind": "signed_in", "actor": "account", "method": "passkey",
            "reason": null, "event": EventId::new(), "trace": null}),
    )
    .await;
    seed(
        "authorization_steps",
        serde_json::json!({"client": "c_charted", "step": "requested"}),
    )
    .await;
    seed(
        "passkeys_used",
        serde_json::json!({"synced": true, "syncable": true, "transport": "internal"}),
    )
    .await;
    let config = Harness::reading(
        AuthConfig::test(&AUTH_ORIGIN.parse().expect("auth origin")),
        Some(reader),
    );
    let harness = Harness::of(config).await;
    let admin = harness.admin().await;
    let dashboard = harness.sign_in(&admin, "/analytics").await;
    let page = html(harness.visit(&dashboard, "/analytics").await).await;
    let now = Utc::now();
    let first_day = (now - Duration::days(29)).format("%b %-d").to_string();
    for shown in [
        "Requests per hour",
        "GET /charted",
        "Sign-ins per day",
        "Synced, built in",
        "Requested",
        "passkeys_used",
        "<code>logs</code>",
        &format!("<th scope=\"row\">{first_day}</th>"),
    ] {
        assert!(page.contains(shown), "{shown} in {page}");
    }
    assert!(
        !page.contains("<code>GET </code>") && !page.contains("9000.0 ms"),
        "an unmatched route in {page}"
    );
}

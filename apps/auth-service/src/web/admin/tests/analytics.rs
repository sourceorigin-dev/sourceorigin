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
    let mut config = AuthConfig::test(&AUTH_ORIGIN.parse().expect("auth origin"));
    config.analytics = Some(stranger);
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
/// request that matched no route, every table's size, `logs` included, and
/// what browsers report: each page's 75th percentile timings, a timing no view
/// measured as a dash, and the errors by kind and script.
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
        "requests",
        serde_json::json!({"service": "auth-service", "instance": "test", "listener": null,
            "method": "GET", "route": "/charted", "status": 503, "duration_us": 1500, "trace": null}),
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
    seed(
        "page_timings",
        serde_json::json!({"route": "/settings/sessions", "ttfb_ms": 90, "fcp_ms": 400,
            "lcp_ms": 1800, "inp_ms": null, "cls": 50}),
    )
    .await;
    seed(
        "script_errors",
        serde_json::json!({"route": "/settings/sessions", "name": "TypeError", "script": "passkey"}),
    )
    .await;
    let mut config = AuthConfig::test(&AUTH_ORIGIN.parse().expect("auth origin"));
    config.analytics = Some(reader);
    let harness = Harness::of(config).await;
    let admin = harness.admin().await;
    let dashboard = harness.sign_in(&admin, "/analytics").await;
    let page = html(harness.visit(&dashboard, "/analytics").await).await;
    let now = Utc::now();
    let first_day = (now - Duration::days(29)).format("%b %-d").to_string();
    let this_hour = now.format("%H:00").to_string();
    for shown in [
        "Requests per hour",
        "GET /charted",
        "Sign-ins per day",
        "Synced, built in",
        "Requested",
        "passkeys_used",
        "<code>logs</code>",
        "<code>/settings/sessions</code>",
        "1.8 s",
        "<td class=\"number\">—</td>",
        "0.05",
        "<code>TypeError</code>",
        "<code>passkey</code>",
        &format!("<th scope=\"row\">{first_day}</th>"),
        "<th scope=\"col\">Server errors</th>",
        &format!("<th scope=\"row\">{this_hour}</th><td>3</td><td>1</td>"),
    ] {
        assert!(page.contains(shown), "{shown} in {page}");
    }
    assert!(
        !page.contains("<code>GET </code>") && !page.contains("9000.0 ms"),
        "an unmatched route in {page}"
    );
}

/// The trace page shows a trace's spans, nested and marked when they failed,
/// and its log lines with their fields, and nothing of another trace's; an ID
/// that isn't one is missing, and without a reader, analytics are off.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and TEST_ANALYTICS_URL: run `just db test`"]
async fn the_trace_page_shows_what_a_trace_did_in_analytics() {
    let owner = Store::test();
    let reader = owner.as_user("trace_reader", "reader-password");
    owner
        .migrate(crate::cli::ANALYTICS_TABLES, None, Some(&reader))
        .await
        .expect("migrated");
    let trace = "4bf92f3577b34da6a3ce929d0e0e4736";
    let time = Utc::now().format("%Y-%m-%d %H:%M:%S%.6f").to_string();
    let span = |trace: &str, span: &str, parent: &str, name: &str, status: &str| {
        serde_json::json!({"time": time, "trace": trace, "span": span, "parent": parent,
            "service": "auth-service", "instance": "test", "name": name, "kind": "server",
            "status": status, "duration_us": 1500, "attributes": {"http.route": "/login"}})
        .to_string()
    };
    let spans = [
        span(trace, "00f067aa0ba902b7", "", "GET /login", "unset"),
        span(
            trace,
            "00f067aa0ba902b8",
            "00f067aa0ba902b7",
            "charted query",
            "error",
        ),
        span(
            "0af7651916cd43dd8448eb211c80319c",
            "00f067aa0ba902b9",
            "",
            "another trace's",
            "unset",
        ),
    ];
    owner
        .insert_rows("spans", spans.join("\n"))
        .await
        .expect("spans");
    let line = serde_json::json!({"time": time, "trace": trace, "span": "00f067aa0ba902b7",
        "service": "auth-service", "instance": "test", "level": "INFO", "target": "auth",
        "body": "event recorded", "attributes": {"event.kind": "signed_in"}});
    owner
        .insert_rows("logs", line.to_string())
        .await
        .expect("logs");
    let mut config = AuthConfig::test(&AUTH_ORIGIN.parse().expect("auth origin"));
    config.analytics = Some(reader);
    let harness = Harness::of(config).await;
    let admin = harness.admin().await;
    let path = format!("/traces/{trace}");
    let dashboard = harness.sign_in(&admin, &path).await;
    let page = html(harness.visit(&dashboard, &path).await).await;
    for shown in [
        "GET /login",
        "charted query (failed)",
        "event recorded",
        "event.kind",
        "http.route",
    ] {
        assert!(page.contains(shown), "{shown} in {page}");
    }
    assert!(
        !page.contains("another trace&#39;s") && !page.contains("another trace's"),
        "{page}"
    );
    let missing = harness.visit(&dashboard, "/traces/not-a-trace").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let off = Harness::new().await;
    let admin = off.admin().await;
    let dashboard = off.sign_in(&admin, &path).await;
    let page = html(off.visit(&dashboard, &path).await).await;
    assert!(page.contains("Analytics are off"), "{page}");
}

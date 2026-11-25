//! What a page's report records: its route template, clamped timings and a few
//! errors, or nothing for a path no page has.

use super::*;
use axum::{body::Body, http::Request};
use serde_json::{Value, json};
use sourceorigin_telemetry::analytics::AnalyticsCapture;
use tower::ServiceExt;

/// A path is recorded only as the page route it is an instance of: a `{param}`
/// segment stands for any one segment, never none, and anything else for
/// itself.
#[test]
fn paths_match_only_their_page_routes() {
    for (path, template) in [
        ("/settings", Some("/settings")),
        (
            "/developers/clients/c_abc/insights",
            Some("/developers/clients/{id}/insights"),
        ),
        (
            "/developers/clients/c_abc",
            Some("/developers/clients/{id}"),
        ),
        ("/developers/clients/", None),
        ("/developers/clients//insights", None),
        ("/developers/clients/c_abc/insights/more", None),
        ("/settings/", None),
        ("/", None),
        ("/account/passkeys", None),
        ("/unknown", None),
        ("", None),
    ] {
        assert_eq!(routes::page::template_of(path), template, "{path}");
    }
}

/// Timings past their limit read as the limit, an unknown script or error kind
/// reads as `other`, and a negative timing, which the script never sends, fails
/// the report.
#[test]
fn a_report_is_clamped_as_it_is_read() {
    let report: PageReport = serde_json::from_value(json!({
        "path": "/settings",
        "lcp": 4_000_000_000_u64,
        "inp": 120,
        "cls": 25_000,
        "errors": [{"name": "MadeUpError", "script": "../../etc"}],
    }))
    .expect("a report");
    assert_eq!(report.lcp, Some(Clamped(MAX_MS)));
    assert_eq!(report.inp, Some(Clamped(120)));
    assert_eq!(report.cls, Some(Clamped(MAX_SHIFT)));
    assert_eq!(report.errors[0].name, ScriptErrorName::Other);
    assert_eq!(report.errors[0].script, ScriptName("other"));
    let negative = serde_json::from_value::<PageReport>(json!({"path": "/settings", "ttfb": -1}));
    negative.expect_err("a negative timing is refused");
}

/// Posts `report` to the route, as the vitals script does, and checks it is
/// answered `204`.
async fn post(report: &Value) {
    let request = Request::post(routes::telemetry::PAGE)
        .header("content-type", "application/json")
        .body(Body::from(report.to_string()))
        .expect("request");
    let response = VitalsApplication::routes()
        .oneshot(request)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

/// A report records one timing row for its page's route, and its first five
/// errors, by kind and script: never its path. A report from a path that isn't
/// one of auth's pages records nothing.
#[tokio::test]
async fn a_report_records_its_route_and_a_foreign_path_nothing() {
    let analytics = AnalyticsCapture::start();
    let error = json!({"name": "TypeError", "script": "passkey"});
    post(&json!({
        "path": "/developers/clients/c_secret_id/settings",
        "ttfb": 80, "fcp": 300, "lcp": 1200, "cls": 50,
        "errors": vec![error; 7],
    }))
    .await;
    assert_eq!(
        analytics.rows("page_timings"),
        [
            json!({"route": "/developers/clients/{id}/settings", "ttfb_ms": 80, "fcp_ms": 300,
            "lcp_ms": 1200, "inp_ms": null, "cls": 50})
        ]
    );
    let errors = analytics.rows("script_errors");
    assert_eq!(errors.len(), MAX_ERRORS);
    assert_eq!(
        errors[0],
        json!({"route": "/developers/clients/{id}/settings", "name": "TypeError", "script": "passkey"})
    );

    let analytics = AnalyticsCapture::start();
    post(&json!({
        "path": "/someone@example.com",
        "lcp": 1200,
        "errors": [{"name": "TypeError", "script": "passkey"}],
    }))
    .await;
    assert_eq!(analytics.rows("page_timings"), Vec::<Value>::new());
    assert_eq!(analytics.rows("script_errors"), Vec::<Value>::new());
}

//! The dashboard's Analytics page, at `/analytics`: what the analytics database
//! counts, as charts with their tables.

use super::AdminApplication as App;
use crate::admin::{AdminError, Analytics};
use chrono::DateTime;
use sourceorigin_common::ErrorChain;
use sourceorigin_ui::{AnalyticsPage, AnalyticsReport, SlowRoute, StoredTable};
use topcoat::{
    Result,
    context::Cx,
    router::{StatusCode, response::Response, route},
    view::view,
};

/// `/analytics`: the charts, or why there are none. A database that doesn't
/// answer is said on the page, which the rest of the dashboard never needs.
#[route(GET "/analytics")]
pub(super) async fn analytics(cx: &Cx) -> Result<Response> {
    let admin = App::admin(cx).await?;
    let app = App::of(cx);
    let page = match &app.analytics {
        None => AnalyticsPage::Off,
        Some(reader) => match app.service.analytics(&admin.operator, reader).await {
            Ok(analytics) => AnalyticsPage::Shown(Box::new(report(analytics))),
            Err(AdminError::Analytics(error)) => {
                tracing::warn!(error = %ErrorChain(&error), "the analytics page could not be read");
                AnalyticsPage::Unavailable
            }
            Err(error) => return Err(error.into()),
        },
    };
    let view = view! { cx => sourceorigin_ui::admin_analytics(page: page) };
    Box::pin(App::render(cx, StatusCode::OK, "Analytics", view)).await
}

/// The page's rows of `read`.
fn report(read: Analytics) -> AnalyticsReport {
    let label = |at: i64, format: &str| {
        DateTime::from_timestamp(at, 0).map_or_else(String::new, |at| at.format(format).to_string())
    };
    AnalyticsReport {
        hours: read
            .requests
            .iter()
            .map(|hour| label(hour.hour, "%H:00"))
            .collect(),
        requests: read
            .requests
            .iter()
            .map(|hour| f64::from(hour.requests))
            .collect(),
        server_errors: read
            .requests
            .iter()
            .map(|hour| f64::from(hour.errors))
            .collect(),
        slowest: read
            .slowest
            .into_iter()
            .map(|route| SlowRoute {
                service: route.service,
                listener: route.listener,
                method: route.method,
                route: route.route,
                requests: route.requests,
                median_ms: route.median_ms,
                p95_ms: route.p95_ms,
            })
            .collect(),
        days: read
            .sign_ins
            .iter()
            .map(|day| label(day.day, "%b %-d"))
            .collect(),
        signed_in: read
            .sign_ins
            .iter()
            .map(|day| f64::from(day.signed_in))
            .collect(),
        failed: read
            .sign_ins
            .iter()
            .map(|day| f64::from(day.failed))
            .collect(),
        steps: read
            .steps
            .iter()
            .map(|counted| (counted.step, f64::from(counted.count)))
            .collect(),
        passkeys: read
            .passkeys
            .iter()
            .map(|kind| (kind.synced, kind.transport, f64::from(kind.count)))
            .collect(),
        tables: read
            .tables
            .into_iter()
            .map(|table| StoredTable {
                name: table.table,
                rows: table.rows,
                bytes: table.bytes,
            })
            .collect(),
    }
}

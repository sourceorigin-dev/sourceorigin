//! The dashboard's Analytics page: what the analytics database counts, as
//! charts with their tables.

use crate::{
    charts::{Point, Series, bar_chart, column_chart, compact, line_chart},
    layout::page_head,
};
use sourceorigin_common::admin::{AuthorizationStepKind, PasskeyTransport};
use topcoat::{
    Result,
    view::{View, component, view},
};

/// How long one route of one service takes, as the page lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct SlowRoute {
    /// The service that answered, such as `auth-service`.
    pub service: String,
    /// Its listener, when it has several, such as `admin`.
    pub listener: Option<String>,
    /// The request's method.
    pub method: String,
    /// The route template.
    pub route: String,
    /// Requests to it.
    pub requests: u32,
    /// The median duration, in milliseconds.
    pub median_ms: f64,
    /// The duration 95 in 100 requests finish within, in milliseconds.
    pub p95_ms: f64,
}

/// What one table stores, as the page lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredTable {
    /// Its name.
    pub name: String,
    /// Its rows.
    pub rows: f64,
    /// The bytes it takes, compressed.
    pub bytes: f64,
}

/// What the page reads, in plain rows: the web layer converts the service's.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalyticsReport {
    /// Requests per hour, the last 24, labelled `14:00`.
    pub requests: Vec<Point>,
    /// Server errors among them.
    pub errors: f64,
    /// The slowest routes of the last 24 hours.
    pub slowest: Vec<SlowRoute>,
    /// The last 30 days, labelled `Nov 24`.
    pub days: Vec<String>,
    /// Sign-ins per day.
    pub signed_in: Vec<f64>,
    /// Failed sign-ins per day.
    pub failed: Vec<f64>,
    /// How often each step of signing in to an app was reached; a step that
    /// wasn't may be left out.
    pub steps: Vec<(AuthorizationStepKind, f64)>,
    /// How often each kind of passkey was used, whether synced, most used
    /// first.
    pub passkeys: Vec<(bool, PasskeyTransport, f64)>,
    /// What each table stores, largest first.
    pub tables: Vec<StoredTable>,
}

/// What the page can show.
#[derive(Clone, Debug, PartialEq)]
pub enum AnalyticsPage {
    /// The dashboard has no reader for the analytics database.
    Off,
    /// The analytics database didn't answer.
    Unavailable,
    /// What it counted.
    Shown(Box<AnalyticsReport>),
}

/// `bytes` as a size: `12.4 MiB`.
fn size(bytes: f64) -> String {
    let mut value = bytes;
    for unit in ["B", "KiB", "MiB"] {
        if value < 1024.0 {
            return if unit == "B" {
                format!("{value:.0} {unit}")
            } else {
                format!("{value:.1} {unit}")
            };
        }
        value /= 1024.0;
    }
    format!("{value:.1} GiB")
}

/// What the page calls `step`.
const fn step_name(step: AuthorizationStepKind) -> &'static str {
    match step {
        AuthorizationStepKind::Requested => "Requested",
        AuthorizationStepKind::SignIn => "Sign-in page",
        AuthorizationStepKind::Consent => "Consent page",
        AuthorizationStepKind::Authorized => "Authorized",
        AuthorizationStepKind::Denied => "Denied",
        AuthorizationStepKind::Other => "Other",
    }
}

/// What the page calls a passkey reached over `transport`, `synced` or not.
fn passkey_name(synced: bool, transport: PasskeyTransport) -> String {
    let kept = if synced { "Synced" } else { "One device" };
    let reached = match transport {
        PasskeyTransport::Internal => "built in",
        PasskeyTransport::Hybrid => "on a phone",
        PasskeyTransport::Usb => "USB",
        PasskeyTransport::Nfc => "NFC",
        PasskeyTransport::Ble => "Bluetooth",
        PasskeyTransport::Unknown => "unknown",
    };
    format!("{kept}, {reached}")
}

/// Where `route` was answered: `auth-service`, or `auth-service, admin`.
fn server(route: &SlowRoute) -> String {
    route.listener.as_ref().map_or_else(
        || route.service.clone(),
        |listener| format!("{}, {listener}", route.service),
    )
}

/// The Analytics page.
#[component]
pub async fn admin_analytics(
    /// What the page shows: its charts, or why there are none.
    page: AnalyticsPage,
) -> Result<impl View> {
    let lead = "What the analytics database counts: the last 24 hours of requests, to every server, and 30 days of sign-ins. Times are UTC.";
    Ok(view! {
        page_head(title: "Analytics", lead: lead.to_owned())
        match page {
            AnalyticsPage::Off => {
                <p class="notice">
                    "Analytics are off: the dashboard has no reader for the analytics database. Set ANALYTICS_READER_URL and ANALYTICS_READER_PASSWORD."
                </p>
            }
            AnalyticsPage::Unavailable => {
                <p class="notice error">"The analytics database didn't answer. Auth logs why, as “the analytics page could not be read”."</p>
            }
            AnalyticsPage::Shown(report) => {
                report_view(report: *report)
            }
        }
    })
}

/// The charts and tables of `report`.
#[component]
async fn report_view(
    /// What the analytics database counted.
    report: AnalyticsReport,
) -> Result<impl View> {
    let requests: f64 = report.requests.iter().map(|point| point.value).sum();
    let signed_in: f64 = report.signed_in.iter().sum();
    let failed: f64 = report.failed.iter().sum();
    let stats = [
        ("Requests, 24 hours", compact(requests)),
        ("Server errors, 24 hours", compact(report.errors)),
        ("Sign-ins, 30 days", compact(signed_in)),
        ("Failed sign-ins, 30 days", compact(failed)),
    ];
    Ok(view! {
        <div class="stats">
            for (label, value) in &stats {
                <div class="stat">
                    <span class="label">(*label)</span>
                    <span class="value">(value.as_str())</span>
                </div>
            }
        </div>
        <section class="card" aria-labelledby="requests-heading">
            <header><h2 id="requests-heading">"Requests per hour"</h2></header>
            <div class="body">
                column_chart(caption: "Requests per hour", points: &report.requests, every: 6)
            </div>
        </section>
        <section class="card" aria-labelledby="sign-ins-heading">
            <header><h2 id="sign-ins-heading">"Sign-ins per day"</h2></header>
            <div class="body">
                line_chart(
                    caption: "Sign-ins per day",
                    labels: &report.days,
                    series: vec![
                        Series { name: "Signed in", values: report.signed_in.clone() },
                        Series { name: "Failed", values: report.failed.clone() },
                    ],
                    every: 7,
                )
            </div>
        </section>
        usage_cards(report: &report)
    })
}

/// The page's cards below its charts of time: sign-ins to apps, passkeys, the
/// slowest routes and storage.
#[component]
async fn usage_cards<'a>(
    /// What the analytics database counted.
    report: &'a AnalyticsReport,
) -> Result<impl View> {
    let stored: f64 = report.tables.iter().map(|table| table.bytes).sum();
    let counted = |step: AuthorizationStepKind| -> f64 {
        report
            .steps
            .iter()
            .filter(|(counted, _)| *counted == step)
            .map(|(_, count)| count)
            .sum()
    };
    let steps: Vec<Point> = AuthorizationStepKind::ALL
        .into_iter()
        // A step this release doesn't know shows only once counted.
        .filter(|step| *step != AuthorizationStepKind::Other || counted(*step) > 0.0)
        .map(|step| Point {
            label: step_name(step).to_owned(),
            value: counted(step),
        })
        .collect();
    let passkeys: Vec<Point> = report
        .passkeys
        .iter()
        .map(|(synced, transport, count)| Point {
            label: passkey_name(*synced, *transport),
            value: *count,
        })
        .collect();
    Ok(view! {
        <div class="two">
            <section class="card" aria-labelledby="steps-heading">
                <header><h2 id="steps-heading">"Signing in to apps, 30 days"</h2></header>
                <div class="body">
                    if steps.iter().all(|step| step.value == 0.0) {
                        <p class="empty">"No app sign-in counted yet."</p>
                    } else {
                        bar_chart(caption: "Signing in to apps, by step", points: &steps)
                    }
                </div>
            </section>
            <section class="card" aria-labelledby="passkeys-heading">
                <header><h2 id="passkeys-heading">"Passkeys used, 30 days"</h2></header>
                <div class="body">
                    if passkeys.is_empty() {
                        <p class="empty">"No passkey use counted yet."</p>
                    } else {
                        bar_chart(caption: "Passkeys used, by kind", points: &passkeys)
                    }
                </div>
            </section>
        </div>
        <section class="card" aria-labelledby="slowest-heading">
            <header><h2 id="slowest-heading">"Slowest routes, 24 hours"</h2></header>
            if report.slowest.is_empty() {
                <p class="empty">"No request yet."</p>
            } else {
                <table>
                    <thead>
                        <tr><th scope="col">"Route"</th><th scope="col">"Server"</th><th scope="col">"Requests"</th><th scope="col">"Median"</th><th scope="col">"95th percentile"</th></tr>
                    </thead>
                    <tbody>
                        for row in &report.slowest {
                            <tr>
                                <td><code>(format!("{} {}", row.method, row.route))</code></td>
                                <td>(server(row))</td>
                                <td class="number">(compact(f64::from(row.requests)))</td>
                                <td class="number">(format!("{:.1} ms", row.median_ms))</td>
                                <td class="number">(format!("{:.1} ms", row.p95_ms))</td>
                            </tr>
                        }
                    </tbody>
                </table>
            }
        </section>
        <section class="card" aria-labelledby="storage-heading">
            <header><h2 id="storage-heading">(format!("Storage: {}", size(stored)))</h2></header>
            if report.tables.is_empty() {
                <p class="empty">"No table holds a row yet."</p>
            } else {
            <table>
                <thead>
                    <tr><th scope="col">"Table"</th><th scope="col">"Rows"</th><th scope="col">"On disk"</th></tr>
                </thead>
                <tbody>
                    for table in &report.tables {
                        <tr>
                            <td><code>(table.name.as_str())</code></td>
                            <td class="number">(compact(table.rows))</td>
                            <td class="number">(size(table.bytes))</td>
                        </tr>
                    }
                </tbody>
            </table>
            }
        </section>
    })
}

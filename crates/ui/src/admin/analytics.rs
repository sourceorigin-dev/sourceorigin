//! The dashboard's Analytics page: what the analytics database counts, as
//! charts with their tables.

use crate::{
    charts::{Bar, Charted, Point, Series, bar_chart, column_chart, compact, line_chart},
    clients::step_name,
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

/// How fast one of auth's pages loads, as the page lists it: the 75th
/// percentile of each timing, none when no view measured it.
#[derive(Clone, Debug, PartialEq)]
pub struct SlowPage {
    /// The page's route template.
    pub route: String,
    /// Views that reported.
    pub views: u32,
    /// Largest contentful paint, in milliseconds.
    pub lcp_ms: Option<f64>,
    /// The longest interaction, in milliseconds.
    pub inp_ms: Option<f64>,
    /// Cumulative layout shift.
    pub cls: Option<f64>,
}

/// How often one kind of error was thrown by one script, as the page lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserError {
    /// The error's kind, such as `TypeError`.
    pub name: String,
    /// The script that threw it, such as `passkey`, or `other`.
    pub script: String,
    /// How often.
    pub count: u32,
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
    /// How fast auth's pages loaded in the last 24 hours, slowest first.
    pub pages: Vec<SlowPage>,
    /// The errors browsers hit on auth's pages in the last 24 hours, most
    /// frequent first.
    pub browser_errors: Vec<BrowserError>,
    /// The last 30 days, labelled `Nov 24`.
    pub days: Vec<String>,
    /// Sign-ins per day.
    pub signed_in: Vec<f64>,
    /// Failed sign-ins per day.
    pub failed: Vec<f64>,
    /// How often each step of signing in to an app was reached, in the order
    /// a sign-in passes them.
    pub steps: Vec<(AuthorizationStepKind, f64)>,
    /// How often each kind of passkey was used, whether synced, most used
    /// first.
    pub passkeys: Vec<(bool, PasskeyTransport, f64)>,
    /// What each table stores, largest first.
    pub tables: Vec<StoredTable>,
}

/// What the page can show.
pub type AnalyticsPage = Charted<Box<AnalyticsReport>>;

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

/// `value` as `shown` writes it, or a dash when no view measured it.
fn measured(value: Option<f64>, shown: fn(f64) -> String) -> String {
    value.map_or_else(|| "—".to_owned(), shown)
}

/// A timing of `ms` milliseconds: `850 ms`, or `1.2 s`.
fn timing(ms: f64) -> String {
    if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

/// A layout shift: `0.08`.
fn shift(cls: f64) -> String {
    format!("{cls:.2}")
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
                missing_analytics(unavailable: false)
            }
            AnalyticsPage::Unavailable => {
                missing_analytics(unavailable: true)
            }
            AnalyticsPage::Shown(report) => {
                report_view(report: *report)
            }
        }
    })
}

/// What a dashboard page of analytics says when it shows none: that the
/// database didn't answer, when `unavailable`, else that analytics are off.
#[component]
pub(crate) async fn missing_analytics(
    /// Whether the database didn't answer, rather than analytics being off.
    unavailable: bool,
) -> Result<impl View> {
    Ok(view! {
        if unavailable {
            <p class="notice error">"The analytics database didn't answer. Auth logs why, as “analytics could not be read”."</p>
        } else {
            <p class="notice">
                "Analytics are off: the dashboard has no reader for the analytics database. Set ANALYTICS_READER_URL and ANALYTICS_READER_PASSWORD."
            </p>
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
    let steps: Vec<Bar> = report
        .steps
        .iter()
        .map(|(step, count)| Bar::of(step_name(*step), *count))
        .collect();
    let passkeys: Vec<Bar> = report
        .passkeys
        .iter()
        .map(|(synced, transport, count)| Bar::of(passkey_name(*synced, *transport), *count))
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
        browser_cards(report: report)
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

/// The cards of what browsers report of auth's pages: how fast each loads,
/// slowest first, and the errors their scripts didn't catch.
#[component]
async fn browser_cards<'a>(
    /// What the analytics database counted.
    report: &'a AnalyticsReport,
) -> Result<impl View> {
    Ok(view! {
        <section class="card" aria-labelledby="page-speed-heading">
            <header><h2 id="page-speed-heading">"Page speed, 24 hours"</h2></header>
            if report.pages.is_empty() {
                <p class="empty">"No page view reported yet."</p>
            } else {
                <table>
                    <thead>
                        <tr>
                            <th scope="col">"Page"</th>
                            <th scope="col">"Views"</th>
                            <th scope="col"><abbr title="Largest contentful paint, which 3 in 4 views beat">"LCP"</abbr></th>
                            <th scope="col"><abbr title="The longest interaction, which 3 in 4 views beat">"INP"</abbr></th>
                            <th scope="col"><abbr title="Cumulative layout shift, which 3 in 4 views beat">"CLS"</abbr></th>
                        </tr>
                    </thead>
                    <tbody>
                        for page in &report.pages {
                            <tr>
                                <td><code>(page.route.as_str())</code></td>
                                <td class="number">(compact(f64::from(page.views)))</td>
                                <td class="number">(measured(page.lcp_ms, timing))</td>
                                <td class="number">(measured(page.inp_ms, timing))</td>
                                <td class="number">(measured(page.cls, shift))</td>
                            </tr>
                        }
                    </tbody>
                </table>
            }
        </section>
        <section class="card" aria-labelledby="browser-errors-heading">
            <header><h2 id="browser-errors-heading">"Browser errors, 24 hours"</h2></header>
            if report.browser_errors.is_empty() {
                <p class="empty">"No script error reported."</p>
            } else {
                <table>
                    <thead>
                        <tr><th scope="col">"Error"</th><th scope="col">"Script"</th><th scope="col">"Count"</th></tr>
                    </thead>
                    <tbody>
                        for error in &report.browser_errors {
                            <tr>
                                <td><code>(error.name.as_str())</code></td>
                                <td><code>(error.script.as_str())</code></td>
                                <td class="number">(compact(f64::from(error.count)))</td>
                            </tr>
                        }
                    </tbody>
                </table>
            }
        </section>
    })
}

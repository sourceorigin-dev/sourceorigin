//! The dashboard's trace page: what one request, job run or delivery attempt
//! did, as its spans on a time axis and its log lines.

use super::analytics::missing_analytics;
use crate::{
    charts::{Charted, Interval, interval_chart},
    layout::page_head,
};
use std::collections::BTreeMap;
use topcoat::{
    Result,
    view::{View, component, view},
};

/// One span, as the page shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceSpan {
    /// Where it sits on the time axis.
    pub interval: Interval,
    /// The service that ran it, such as `auth-service`.
    pub service: String,
    /// Its attributes, as text.
    pub attributes: BTreeMap<String, String>,
}

/// One log line, as the page shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceLog {
    /// When it was logged, in milliseconds from the trace's first span or log
    /// line.
    pub at_ms: f64,
    /// `ERROR`, `WARN`, `INFO`, `DEBUG` or `TRACE`.
    pub level: String,
    /// The module that logged it.
    pub target: String,
    /// The message.
    pub body: String,
    /// Its fields, as text.
    pub attributes: BTreeMap<String, String>,
}

/// A trace, in plain rows.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceReport {
    /// Its spans, each after its parent.
    pub spans: Vec<TraceSpan>,
    /// Its log lines, oldest first.
    pub logs: Vec<TraceLog>,
    /// How many of each the page shows at most, when it has more spans,
    /// more log lines, or both.
    pub cut: Option<(usize, bool, bool)>,
}

/// What the trace page can show.
pub type TracePage = Charted<TraceReport>;

/// The trace `id`'s page.
#[component]
pub async fn admin_trace<'a>(
    /// The trace ID, as 32 hex digits.
    id: &'a str,
    /// What the page shows: the trace, or why there is none.
    page: TracePage,
) -> Result<impl View> {
    let lead = format!(
        "Trace {id}: its spans, in the order they nest and started, and its log lines, in milliseconds from its first. Spans are kept 14 days, log lines 30."
    );
    Ok(view! {
        page_head(title: "Trace", lead: lead)
        match page {
            Charted::Off => {
                missing_analytics(unavailable: false)
            }
            Charted::Unavailable => {
                missing_analytics(unavailable: true)
            }
            Charted::Shown(report) => {
                trace_view(report: report)
            }
        }
    })
}

/// The spans and log lines of `report`, or that it has none.
#[component]
async fn trace_view(
    /// The trace's spans and log lines.
    report: TraceReport,
) -> Result<impl View> {
    let intervals: Vec<Interval> = report
        .spans
        .iter()
        .map(|span| span.interval.clone())
        .collect();
    Ok(view! {
        if report.spans.is_empty() && report.logs.is_empty() {
            <p class="notice">"Nothing has this trace ID: its spans are kept 14 days, and its log lines 30."</p>
        } else {
            if let Some((shown, spans, logs)) = report.cut {
                <p class="notice warning">
                    (format!("This trace has more {} than the page shows: it shows the first {shown} of each.", match (spans, logs) {
                        (true, true) => "spans and log lines",
                        (true, false) => "spans",
                        _ => "log lines",
                    }))
                </p>
            }
            <section class="card" aria-labelledby="spans-heading">
                <header><h2 id="spans-heading">"Spans"</h2></header>
                <div class="body">
                    if report.spans.is_empty() {
                        <p class="empty">"No span yet: they arrive a few seconds after their request ends, and are kept 14 days."</p>
                    } else {
                        interval_chart(caption: "Spans, by when they started", intervals: &intervals)
                        <details class="chart-table">
                            <summary>"Show each span's attributes"</summary>
                            for span in &report.spans {
                                <h3>(span.interval.label.as_str()) " · " (span.service.as_str())</h3>
                                attributes(fields: &span.attributes)
                            }
                        </details>
                    }
                </div>
            </section>
            <section class="card" aria-labelledby="logs-heading">
                <header><h2 id="logs-heading">"Log lines"</h2></header>
                if report.logs.is_empty() {
                    <p class="empty">"No log line is kept for this trace."</p>
                } else {
                    <table>
                        <thead>
                            <tr><th scope="col">"At"</th><th scope="col">"Level"</th><th scope="col">"Module"</th><th scope="col">"Message"</th></tr>
                        </thead>
                        <tbody>
                            for line in &report.logs {
                                <tr>
                                    <td class="number">(format!("{:.1} ms", line.at_ms))</td>
                                    <td>(line.level.as_str())</td>
                                    <td><code>(line.target.as_str())</code></td>
                                    <td>
                                        (line.body.as_str())
                                        if !line.attributes.is_empty() {
                                            <details>
                                                <summary>"Fields"</summary>
                                                attributes(fields: &line.attributes)
                                            </details>
                                        }
                                    </td>
                                </tr>
                            }
                        </tbody>
                    </table>
                }
            </section>
        }
    })
}

/// `fields`, as a list of names and values.
#[component]
async fn attributes<'a>(
    /// The names and values to list.
    fields: &'a BTreeMap<String, String>,
) -> Result<impl View> {
    Ok(view! {
        if fields.is_empty() {
            <p class="empty">"None."</p>
        } else {
            <dl class="attributes">
                for (name, value) in fields {
                    <dt><code>(name.as_str())</code></dt>
                    <dd>(value.as_str())</dd>
                }
            </dl>
        }
    })
}

//! The dashboard's trace page, at `/traces/{trace}`: one trace's spans and log
//! lines, which each listed event links by its trace ID.

use super::AdminApplication as App;
use crate::admin::SHOWN;
use sourceorigin_common::TraceId;
use sourceorigin_ui::{Interval, TraceLog, TraceReport, TraceSpan};
use topcoat::{
    Result,
    context::Cx,
    router::{StatusCode, path_param, response::Response, route},
    view::view,
};

path_param!(pub trace: TraceId, error = not_found);

/// `/traces/{trace}`: the trace's spans and log lines, or why there are none.
#[route(GET "/traces/{trace}")]
pub(super) async fn trace(cx: &Cx) -> Result<Response> {
    let admin = App::admin(cx).await?;
    let app = App::of(cx);
    let id = *path_param::<Trace>(cx)?;
    let page = app
        .charted("trace", async |reader| {
            app.service.trace(&admin.operator, reader, id).await
        })
        .await?
        .map(report);
    let id = id.to_string();
    let view = view! { cx => sourceorigin_ui::admin_trace(id: &id, page: page) };
    Box::pin(App::render(cx, StatusCode::OK, "Trace", view)).await
}

/// `us` microseconds as milliseconds: a whole number of them, up to 49
/// days', and the fraction.
fn millis(us: i64) -> f64 {
    let whole = u32::try_from(us.div_euclid(1000)).unwrap_or(u32::MAX);
    let fraction = u32::try_from(us.rem_euclid(1000)).unwrap_or(0);
    f64::from(whole) + f64::from(fraction) / 1000.0
}

/// The page's rows of `read`, its times from its first span or log line.
fn report(read: crate::admin::Trace) -> TraceReport {
    let first = read
        .spans
        .iter()
        .map(|(_, span)| span.start_us)
        .chain(read.logs.iter().map(|line| line.at_us))
        .min()
        .unwrap_or(0);
    TraceReport {
        spans: read
            .spans
            .into_iter()
            .map(|(depth, span)| TraceSpan {
                interval: Interval {
                    label: span.name,
                    depth,
                    start: millis(span.start_us - first),
                    length: millis(i64::try_from(span.duration_us).unwrap_or(i64::MAX)),
                    failed: span.status == "error",
                },
                service: span.service,
                attributes: span.attributes,
            })
            .collect(),
        logs: read
            .logs
            .into_iter()
            .map(|line| TraceLog {
                at_ms: millis(line.at_us - first),
                level: line.level,
                target: line.target,
                body: line.body,
                attributes: line.attributes,
            })
            .collect(),
        cut: (read.more_spans || read.more_logs).then_some((
            SHOWN,
            read.more_spans,
            read.more_logs,
        )),
    }
}

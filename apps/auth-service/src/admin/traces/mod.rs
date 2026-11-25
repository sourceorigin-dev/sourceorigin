//! The dashboard's trace page: what one request, job run or delivery attempt
//! did, as its spans and log lines, read from the analytics database as the
//! dashboard's reader. See `docs/dev/operations/analytics.md`.

use super::{AdminError, AdminService, Operator};
use serde::Deserialize;
use sourceorigin_common::TraceId;
use sourceorigin_telemetry::analytics::Store;
use std::collections::BTreeMap;

/// The most spans, and the most log lines, a trace page shows: a request has
/// a few dozen, a long job run can have more.
pub(crate) const SHOWN: usize = 1000;

/// One span of a trace.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, clickhouse::Row)]
pub(crate) struct Span {
    /// When it started, in microseconds since the epoch.
    pub(crate) start_us: i64,
    /// Its ID.
    pub(crate) id: String,
    /// Its parent's ID, or empty for a root span.
    pub(crate) parent: String,
    /// The service, such as `auth-service`.
    pub(crate) service: String,
    /// Its name, such as `GET /login`.
    pub(crate) name: String,
    /// `error` when it failed, else `unset` or `ok`.
    pub(crate) status: String,
    /// How long it took, in microseconds.
    pub(crate) duration_us: u64,
    /// Its attributes, as text.
    pub(crate) attributes: BTreeMap<String, String>,
}

/// One log line of a trace.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, clickhouse::Row)]
pub(crate) struct LogLine {
    /// When it was logged, in microseconds since the epoch.
    pub(crate) at_us: i64,
    /// `ERROR`, `WARN`, `INFO`, `DEBUG` or `TRACE`.
    pub(crate) level: String,
    /// The module that logged it.
    pub(crate) target: String,
    /// The message.
    pub(crate) body: String,
    /// Its fields, as text.
    pub(crate) attributes: BTreeMap<String, String>,
}

/// What the trace page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Trace {
    /// Its spans, each after its parent and its earlier siblings, with how deep
    /// it is: a root span is 0.
    pub(crate) spans: Vec<(usize, Span)>,
    /// Its log lines, oldest first.
    pub(crate) logs: Vec<LogLine>,
    /// Whether it has more spans than the page shows, [`SHOWN`].
    pub(crate) more_spans: bool,
    /// Whether it has more log lines than the page shows.
    pub(crate) more_logs: bool,
}

impl Trace {
    /// `spans`, as [`Trace::spans`] orders them. A span whose parent isn't
    /// among them, as when the parent is past its retention or past
    /// [`SHOWN`], is a root, and so is any span no root leads to, such as one
    /// in a cycle of parents, so every span shows once.
    fn tree(spans: Vec<Span>) -> Vec<(usize, Span)> {
        let ids: Vec<&str> = spans.iter().map(|span| span.id.as_str()).collect();
        let roots: Vec<usize> = (0..spans.len())
            .filter(|&i| !ids.contains(&spans[i].parent.as_str()))
            .collect();
        let mut spans: Vec<Option<Span>> = spans.into_iter().map(Some).collect();
        let mut order = Vec::with_capacity(spans.len());
        let mut pending: Vec<(usize, usize)> = roots.into_iter().rev().map(|i| (0, i)).collect();
        loop {
            while let Some((depth, i)) = pending.pop() {
                let Some(span) = spans[i].take() else {
                    continue;
                };
                let children = (0..spans.len()).rev().filter(|&child| {
                    spans[child]
                        .as_ref()
                        .is_some_and(|child| child.parent == span.id)
                });
                pending.extend(children.map(|child| (depth + 1, child)));
                order.push((depth, span));
            }
            match spans.iter().position(Option::is_some) {
                Some(unreached) => pending.push((0, unreached)),
                None => return order,
            }
        }
    }
}

impl AdminService {
    /// Reads the trace `id` through `reader`, ClickHouse's read-only user. It
    /// has no span and no log line once both are past their retention.
    ///
    /// # Errors
    /// Fails when the operator may not read, or ClickHouse refuses or doesn't
    /// answer a query.
    pub(crate) async fn trace(
        &self,
        operator: &Operator,
        reader: &Store,
        id: TraceId,
    ) -> Result<Trace, AdminError> {
        operator.authorize_read(&self.database).await?;
        let id = id.to_string();
        let (mut spans, mut logs): (Vec<Span>, Vec<LogLine>) = tokio::try_join!(
            reader
                .query(
                    "SELECT toUnixTimestamp64Micro(time) AS start_us, toString(span) AS id, parent,
                        service, name, status, duration_us, attributes
                    FROM spans WHERE trace = {trace:FixedString(32)}
                    ORDER BY time LIMIT {read:UInt32}",
                )
                .param("trace", id.as_str())
                .param("read", SHOWN + 1)
                .rows(),
            reader
                .query(
                    "SELECT toUnixTimestamp64Micro(time) AS at_us, level, target, body, attributes
                    FROM logs WHERE trace = {trace:FixedString(32)}
                    ORDER BY time LIMIT {read:UInt32}",
                )
                .param("trace", id.as_str())
                .param("read", SHOWN + 1)
                .rows(),
        )?;
        let (more_spans, more_logs) = (spans.len() > SHOWN, logs.len() > SHOWN);
        spans.truncate(SHOWN);
        logs.truncate(SHOWN);
        Ok(Trace {
            spans: Trace::tree(spans),
            logs,
            more_spans,
            more_logs,
        })
    }
}

#[cfg(test)]
mod tests;

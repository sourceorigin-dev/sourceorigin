//! Spans and log records, written to ClickHouse's `spans` and `logs` tables as
//! the SDK's batch processors hand them over.

use crate::analytics::Store;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use opentelemetry::{
    KeyValue, Value,
    logs::AnyValue,
    trace::{SpanId, SpanKind, Status, TraceId},
};
use opentelemetry_sdk::{
    error::{OTelSdkError, OTelSdkResult},
    logs::{LogBatch, LogExporter},
    trace::{SpanData, SpanExporter},
};
use serde::Serialize;
use sourceorigin_common::{ErrorChain, config::ConfigurationError};
use std::{collections::BTreeMap, fmt, sync::Arc, time::SystemTime};
use thiserror::Error;
use tokio::runtime::{Handle, TryCurrentError};

/// What [`Telemetry::init`](crate::Telemetry::init) does when the analytics
/// database is configured but export cannot start, such as on a malformed
/// `ANALYTICS_URL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Export {
    /// Fail: a server must not run without the telemetry its alerts read, so a
    /// broken configuration stops the deployment instead.
    Required,
    /// Log to stderr only, with a warning: a command-line run, such as a
    /// migration or an account repair, must work while telemetry is broken.
    Optional,
}

/// Why export could not start.
#[derive(Debug, Error)]
pub enum ExportError {
    /// `ANALYTICS_URL` or `ANALYTICS_PASSWORD` is invalid.
    #[error(transparent)]
    Configuration(#[from] ConfigurationError),
    /// Telemetry was installed outside a tokio runtime, which sends the rows.
    #[error("telemetry export needs a tokio runtime")]
    Runtime(#[from] TryCurrentError),
}

/// Attribute keys never stored, whoever sets them: they can hold a path, a
/// query string, an address or a header. Auth never records them; this is the
/// second line of defense.
const REDACTED: [&str; 4] = ["url.path", "url.query", "url.full", "client.address"];
/// Attribute key prefixes never stored, for the same reason.
const REDACTED_PREFIXES: [&str; 2] = ["http.request.header.", "http.response.header."];

/// The tables spans and log records are stored in, which `migrate` creates
/// beside the signals': kept for days, not months, and holding text, so they
/// are not signals. Trace IDs have a bloom filter, so a lookup by trace reads
/// little.
pub(crate) const TABLES: [&str; 2] = [
    "CREATE TABLE IF NOT EXISTS spans (
    time DateTime64(6, 'UTC') CODEC(DoubleDelta, ZSTD) COMMENT 'When the span started.',
    trace FixedString(32) COMMENT 'Its trace.',
    span FixedString(16) COMMENT 'Its ID.',
    parent String COMMENT 'Its parent span, or empty for a root span.',
    service LowCardinality(String) COMMENT 'The service, such as `auth-service`.',
    instance LowCardinality(String) COMMENT 'The process, a new UUID each start.',
    name LowCardinality(String) COMMENT 'Its name, such as `GET /login`.',
    kind LowCardinality(String) COMMENT '`server`, `client` or `internal`.',
    status LowCardinality(String) COMMENT '`error` when it failed, else `unset` or `ok`.',
    duration_us UInt64 COMMENT 'How long it took, in microseconds.',
    attributes Map(LowCardinality(String), String) COMMENT 'Its attributes, as text.',
    INDEX trace_index trace TYPE bloom_filter GRANULARITY 1
) ENGINE = MergeTree
PARTITION BY toYYYYMMDD(time)
ORDER BY (service, name, time)
TTL toDateTime(time) + INTERVAL 14 DAY
SETTINGS ttl_only_drop_parts = 1
COMMENT 'Every span of auth and sourceorigin.dev: a trace per request, job run and delivery attempt.'",
    "CREATE TABLE IF NOT EXISTS logs (
    time DateTime64(6, 'UTC') CODEC(DoubleDelta, ZSTD) COMMENT 'When it was logged.',
    trace FixedString(32) COMMENT 'The trace it was logged in, or zeros.',
    span FixedString(16) COMMENT 'The span it was logged in, or zeros.',
    service LowCardinality(String) COMMENT 'The service, such as `auth-service`.',
    instance LowCardinality(String) COMMENT 'The process, a new UUID each start.',
    level LowCardinality(String) COMMENT '`ERROR`, `WARN`, `INFO` or `DEBUG`.',
    target LowCardinality(String) COMMENT 'The module that logged it.',
    body String COMMENT 'The message.',
    attributes Map(LowCardinality(String), String) COMMENT 'Its fields, as text.',
    INDEX trace_index trace TYPE bloom_filter GRANULARITY 1
) ENGINE = MergeTree
PARTITION BY toYYYYMMDD(time)
ORDER BY (service, level, time)
TTL toDateTime(time) + INTERVAL 30 DAY
SETTINGS ttl_only_drop_parts = 1
COMMENT 'Every log line of auth and sourceorigin.dev that stderr shows at info or above.'",
];

/// Who wrote a span or log record: the columns every row of both tables has.
#[derive(Clone)]
pub(crate) struct Source {
    /// Sends the rows.
    store: Store,
    /// The runtime the inserts run on: the SDK exports from threads of its
    /// own, which have none for `reqwest`.
    runtime: Handle,
    /// `service.name`.
    service: Arc<str>,
    /// `service.instance.id`.
    instance: Arc<str>,
}

impl Source {
    /// Writes as `service` and `instance` to `store`, on the current runtime.
    pub(crate) fn new(store: Store, service: &str, instance: &str) -> Result<Self, ExportError> {
        Ok(Self {
            store,
            runtime: Handle::try_current()?,
            service: Arc::from(service),
            instance: Arc::from(instance),
        })
    }

    /// The store rows go to, which analytics shares.
    pub(crate) fn store(&self) -> Store {
        self.store.clone()
    }

    /// `rows`, one JSON object per line, serialized at once, so the insert
    /// borrows nothing of the batch.
    fn lines<R: Serialize>(rows: impl Iterator<Item = R>) -> String {
        rows.filter_map(|row| serde_json::to_string(&row).ok())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Inserts `body`, rows of `table`, on the runtime.
    fn insert(
        &self,
        table: &'static str,
        body: String,
    ) -> impl Future<Output = OTelSdkResult> + Send + use<> {
        let store = self.store.clone();
        let sent = self
            .runtime
            .spawn(async move { store.insert(table, Bytes::from(body)).await });
        async move {
            let failed = |error: &dyn std::error::Error| {
                OTelSdkError::InternalFailure(ErrorChain(error).to_string())
            };
            match sent.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(failed(&error)),
                Err(error) => Err(failed(&error)),
            }
        }
    }

    /// `time` as ClickHouse parses a `DateTime64(6)`.
    fn time(time: SystemTime) -> String {
        DateTime::<Utc>::from(time)
            .format("%Y-%m-%d %H:%M:%S%.6f")
            .to_string()
    }

    /// `attributes` as text, without the redacted ones.
    fn attributes<'a>(
        attributes: impl Iterator<Item = (&'a str, String)>,
    ) -> BTreeMap<&'a str, String> {
        attributes
            .filter(|(key, _)| {
                !REDACTED.contains(key)
                    && !REDACTED_PREFIXES
                        .iter()
                        .any(|prefix| key.starts_with(prefix))
            })
            .collect()
    }
}

/// One row of `spans`.
#[derive(Serialize)]
struct SpanRow<'a> {
    /// When it started.
    time: String,
    /// Its trace, as 32 hex digits.
    trace: String,
    /// Its ID, as 16 hex digits.
    span: String,
    /// Its parent's ID, or empty.
    parent: String,
    /// The service.
    service: &'a str,
    /// The process.
    instance: &'a str,
    /// Its name.
    name: &'a str,
    /// Its kind.
    kind: &'static str,
    /// Whether it failed.
    status: &'static str,
    /// How long it took.
    duration_us: u64,
    /// Its attributes.
    attributes: BTreeMap<&'a str, String>,
}

/// Spans, as `spans` rows.
#[derive(Debug)]
pub(crate) struct SpanRows(pub(crate) Source);

impl SpanRows {
    /// `span`'s row.
    fn row<'a>(&'a self, span: &'a SpanData) -> SpanRow<'a> {
        let duration = span
            .end_time
            .duration_since(span.start_time)
            .unwrap_or_default();
        SpanRow {
            time: Source::time(span.start_time),
            trace: span.span_context.trace_id().to_string(),
            span: span.span_context.span_id().to_string(),
            parent: if span.parent_span_id == SpanId::INVALID {
                String::new()
            } else {
                span.parent_span_id.to_string()
            },
            service: &self.0.service,
            instance: &self.0.instance,
            name: &span.name,
            kind: match span.span_kind {
                SpanKind::Server => "server",
                SpanKind::Client => "client",
                SpanKind::Producer => "producer",
                SpanKind::Consumer => "consumer",
                SpanKind::Internal => "internal",
            },
            status: match span.status {
                Status::Unset => "unset",
                Status::Ok => "ok",
                Status::Error { .. } => "error",
            },
            duration_us: u64::try_from(duration.as_micros()).unwrap_or(u64::MAX),
            attributes: Source::attributes(
                span.attributes
                    .iter()
                    .map(|KeyValue { key, value, .. }| (key.as_str(), Self::text(value))),
            ),
        }
    }

    /// An attribute's value as text.
    fn text(value: &Value) -> String {
        value.as_str().into_owned()
    }
}

impl SpanExporter for SpanRows {
    /// Inserts the batch as one insert.
    fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        let body = Source::lines(batch.iter().map(|span| self.row(span)));
        self.0.insert("spans", body)
    }
}

/// One row of `logs`.
#[derive(Serialize)]
struct LogRow<'a> {
    /// When it was logged.
    time: String,
    /// Its trace, or zeros.
    trace: String,
    /// Its span, or zeros.
    span: String,
    /// The service.
    service: &'a str,
    /// The process.
    instance: &'a str,
    /// Its level.
    level: &'static str,
    /// The module that logged it.
    target: &'a str,
    /// Its message.
    body: String,
    /// Its fields.
    attributes: BTreeMap<&'a str, String>,
}

/// Log records, as `logs` rows.
#[derive(Debug)]
pub(crate) struct LogRows(pub(crate) Source);

impl LogRows {
    /// A field's value as text.
    fn text(value: &AnyValue) -> String {
        match value {
            AnyValue::String(text) => text.as_str().to_owned(),
            AnyValue::Int(number) => number.to_string(),
            AnyValue::Double(number) => number.to_string(),
            AnyValue::Boolean(flag) => flag.to_string(),
            other => format!("{other:?}"),
        }
    }
}

impl LogExporter for LogRows {
    /// Inserts the batch as one insert.
    fn export(&self, batch: LogBatch<'_>) -> impl Future<Output = OTelSdkResult> + Send {
        let source = &self.0;
        let rows = batch.iter().map(|(record, _)| {
            let context = record.trace_context();
            LogRow {
                time: Source::time(
                    record
                        .timestamp()
                        .or(record.observed_timestamp())
                        .unwrap_or_else(SystemTime::now),
                ),
                trace: context
                    .map_or(TraceId::INVALID, |context| context.trace_id)
                    .to_string(),
                span: context
                    .map_or(SpanId::INVALID, |context| context.span_id)
                    .to_string(),
                service: &source.service,
                instance: &source.instance,
                level: record.severity_text().unwrap_or("INFO"),
                target: record.target().map_or("", |target| target.as_ref()),
                body: record.body().map_or_else(String::new, Self::text),
                attributes: Source::attributes(
                    record
                        .attributes_iter()
                        .map(|(key, value)| (key.as_str(), Self::text(value))),
                ),
            }
        });
        let body = Source::lines(rows);
        source.insert("logs", body)
    }
}

impl fmt::Debug for Source {
    /// Names the service and instance only.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Source")
            .field("service", &self.service)
            .field("instance", &self.instance)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;

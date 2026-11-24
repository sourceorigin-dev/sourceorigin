//! What reaches the `spans` and `logs` tables.

use super::*;
use opentelemetry::{
    logs::{LogRecord as _, Logger as _, LoggerProvider as _},
    trace::{Span as _, Tracer as _, TracerProvider as _},
};
use opentelemetry_sdk::{logs::SdkLoggerProvider, trace::SdkTracerProvider};

/// A span and a log record reach their tables, without the attributes that
/// can hold a path, an address or a header.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs TEST_ANALYTICS_URL, a scratch ClickHouse: run `just analytics test`"]
async fn analytics_spans_and_logs_reach_clickhouse() {
    let store = Store::test();
    store.migrate(&[], None).await.expect("migrated");
    let source = Source::new(store.clone(), "exported", "instance").expect("a runtime");
    let tracer = SdkTracerProvider::builder()
        .with_simple_exporter(SpanRows(source.clone()))
        .build();
    let mut span = tracer.tracer("test").start("GET /items/{id}");
    span.set_attribute(KeyValue::new("http.route", "/items/{id}"));
    span.set_attribute(KeyValue::new("url.path", "/items/secret"));
    span.set_attribute(KeyValue::new("http.request.header.cookie", "secret"));
    span.end();
    let logger = SdkLoggerProvider::builder()
        .with_simple_exporter(LogRows(source))
        .build();
    let log = logger.logger("test");
    let mut record = log.create_log_record();
    record.set_body("event recorded".into());
    record.add_attribute("client.address", "192.0.2.1");
    record.add_attribute("event.kind", "signed_in");
    log.emit(record);
    let spans = store
        .select(
            "SELECT name, attributes['http.route'], length(attributes) FROM spans \
             WHERE service = 'exported'",
        )
        .await
        .expect("spans");
    assert_eq!(spans, "GET /items/{id}\t/items/{id}\t1");
    let logs = store
        .select("SELECT body, attributes['event.kind'], length(attributes) FROM logs WHERE service = 'exported'")
        .await
        .expect("logs");
    assert_eq!(logs, "event recorded\tsigned_in\t1");
}

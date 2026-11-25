//! How signals declare their tables, and what `migrate` runs for them.

use super::Signal;
use sourceorigin_common::{ClientId, EventId, TraceId};

crate::signal! {
    /// A request refused by a rate limit, with a column of every type.
    struct Throttled in "throttled", kept 30 days, ordered by (category) {
        /// Its rate category.
        category: &'static str,
        /// How many requests it refused.
        refused: u64,
        /// Whether it was the first.
        first: bool,
        /// A byte.
        tiny: u8,
        /// A short.
        short: u16,
        /// A word.
        word: u32,
        /// A signed number.
        signed: i64,
        /// A fraction.
        fraction: f64,
        /// A vocabulary value, if any.
        maybe: Option<&'static str>,
        /// A client.
        client: ClientId,
        /// A client, if any.
        maybe_client: Option<ClientId>,
        /// An event.
        event: EventId,
        /// A trace, if any.
        trace: Option<TraceId>,
    }
}

impl Throttled {
    /// A row with every column set.
    fn new(category: &'static str, refused: u64) -> Self {
        Self {
            category,
            refused,
            first: true,
            tiny: 1,
            short: 2,
            word: 3,
            signed: -4,
            fraction: 0.5,
            maybe: Some("value"),
            client: ClientId::generated(&[1; 32]),
            maybe_client: Some(ClientId::generated(&[2; 32])),
            event: EventId::new(),
            trace: TraceId::from_bytes([7; 16]),
        }
    }
}

/// A table's statements create it with every column, sorted by its keys then its
/// time, and then bring an existing table to the declaration.
#[test]
fn a_signal_declares_its_table() {
    insta::assert_snapshot!(Throttled::TABLE.statements().join(";\n\n"));
}

/// A capture sees a recorded row's columns, without its time.
#[test]
fn a_capture_sees_recorded_rows() {
    let capture = super::AnalyticsCapture::start();
    let row = Throttled::new("login", 2);
    row.record();
    let rows = capture.rows("throttled");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["category"], "login");
    assert_eq!(rows[0]["client"], row.client.as_str());
    assert_eq!(rows[0]["trace"], "07".repeat(16));
}

/// A product row keeps only its day.
#[test]
fn a_product_row_keeps_only_its_day() {
    use super::writer::Stamp;
    assert!(Stamp::Day.now().ends_with(" 00:00:00.000"));
    assert_eq!(
        Stamp::Millisecond.now().len(),
        "2026-01-01 00:00:00.000".len()
    );
}

/// A product table's rows wait for the hour, and then go first.
#[test]
fn product_rows_wait_for_the_hour_then_go_first() {
    use super::writer::{Batches, Stamp, Writer};
    let row = || vec!["{}".to_owned()];
    let mut batches: Batches = [
        ("requests", (Stamp::Millisecond, row())),
        ("passkeys_used", (Stamp::Day, row())),
    ]
    .into();
    let tables = |due: Vec<(&'static str, Vec<String>)>| {
        due.into_iter().map(|(table, _)| table).collect::<Vec<_>>()
    };
    assert_eq!(tables(Writer::due(&mut batches, false)), ["requests"]);
    batches.get_mut("requests").expect("requests").1 = row();
    assert_eq!(
        tables(Writer::due(&mut batches, true)),
        ["passkeys_used", "requests"]
    );
}

/// A table at its limit drops its own rows, while another table's still queue.
#[tokio::test]
async fn a_full_table_drops_only_its_own_rows() {
    use super::writer::{DROPPED, QUEUE, QUEUED};
    use std::sync::atomic::Ordering;
    let unreachable = super::Store::new(
        sourceorigin_common::config::Variable::new("TEST"),
        "http://writer@127.0.0.1:9/analytics",
        "password".into(),
    )
    .expect("a valid URL");
    let _writer = super::Writer::start(unreachable).expect("the first writer");
    for _ in 0..=QUEUE {
        Throttled::new("login", 1).record();
    }
    let other = EventId::new();
    Other { event: other }.record();
    let queued = QUEUED.lock().expect("unpoisoned").clone();
    assert_eq!(queued["throttled"], QUEUE);
    assert_eq!(queued["other"], 1);
    assert_eq!(DROPPED.load(Ordering::Relaxed), 1);
}

crate::signal! {
    /// Another table's row.
    struct Other in "other", kept 1 days, ordered by () {
        /// An event.
        event: EventId,
    }
}

/// A row the reader reads back.
#[derive(serde::Deserialize)]
struct Refused {
    /// Requests refused.
    refused: u64,
}

/// `migrate` creates a signal's table, again without error, a writer that may
/// insert rows but read none, and a reader that may read them, bound to its
/// parameters, spans included, but insert none; a recorded row reaches the
/// table once flushed.
#[tokio::test]
#[ignore = "needs TEST_ANALYTICS_URL, a scratch ClickHouse: run `just analytics test`"]
async fn migrate_provisions_a_writer_that_only_inserts_and_a_reader_that_only_reads() {
    let owner = super::Store::test();
    let writer = owner.as_user("test_writer", "writer-password");
    let reader = owner.as_user("test_reader", "reader-password");
    for _ in 0..2 {
        owner
            .migrate(&[Throttled::TABLE], Some(&writer), Some(&reader))
            .await
            .expect("migrated");
    }
    let started = super::Writer::start(writer.clone()).expect("the first writer");
    Throttled::new("login", 3).record();
    started.flush().await;
    let stored = owner
        .select("SELECT category, refused FROM throttled")
        .await
        .expect("selected");
    assert_eq!(stored, "login\t3");
    assert!(
        writer
            .select("SELECT count() FROM throttled")
            .await
            .is_err(),
        "the writer may not read"
    );
    let read: Vec<Refused> = reader
        .query(
            "SELECT refused FROM throttled WHERE category = {category:String}",
            &[("category", "login")],
        )
        .await
        .expect("read");
    assert_eq!(read.iter().map(|row| row.refused).collect::<Vec<_>>(), [3]);
    let inserted = reader
        .insert("throttled", bytes::Bytes::from_static(b"{}"))
        .await;
    assert!(inserted.is_err(), "the reader may not insert");
    assert!(
        reader.select("SELECT count() FROM spans").await.is_ok(),
        "the reader reads what one request did, for the trace page"
    );
}

//! How signals declare their tables, and what `migrate` runs for them.

use super::{Signal, Table};

crate::signal! {
    /// A request refused by a rate limit.
    struct Throttled in "throttled", kept 30 days, ordered by (category) {
        /// Its rate category.
        category: &'static str,
        /// How many requests it refused.
        refused: u64,
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
    Throttled {
        category: "login",
        refused: 2,
    }
    .record();
    assert_eq!(
        capture.rows("throttled"),
        [serde_json::json!({ "category": "login", "refused": 2 })]
    );
}

/// The declaration keeps the doc comments as the table's comment.
#[test]
fn a_table_keeps_its_description() {
    let Table { description, .. } = Throttled::TABLE;
    assert_eq!(description, [" A request refused by a rate limit."]);
}

/// `migrate` creates a signal's table, again without error, and a writer that
/// may insert rows but read none; a recorded row reaches the table once flushed.
#[tokio::test]
#[ignore = "needs TEST_ANALYTICS_URL, a scratch ClickHouse: run `just analytics test`"]
async fn migrate_provisions_a_writer_that_only_inserts() {
    let owner = super::Store::test();
    let writer = owner.as_user("test_writer", "writer-password");
    for _ in 0..2 {
        owner
            .migrate(&[Throttled::TABLE], Some(&writer))
            .await
            .expect("migrated");
    }
    let started = super::Writer::start(writer.clone()).expect("the first writer");
    Throttled {
        category: "login",
        refused: 3,
    }
    .record();
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
}

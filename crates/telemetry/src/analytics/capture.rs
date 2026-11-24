//! Rows a test records, captured on its thread instead of queued.

use serde::Serialize;
use std::cell::RefCell;

thread_local! {
    /// The rows recorded on this thread since a capture started, by table.
    static CAPTURED: RefCell<Option<Vec<(&'static str, serde_json::Value)>>> =
        const { RefCell::new(None) };
}

/// Captures the rows this thread records until dropped, as
/// [`Telemetry::capture`](crate::Telemetry::capture) does spans: a test on a
/// current-thread runtime captures every row its requests record.
pub struct AnalyticsCapture(());

impl AnalyticsCapture {
    /// Starts capturing this thread's rows.
    #[must_use]
    pub fn start() -> Self {
        CAPTURED.with_borrow_mut(|captured| *captured = Some(Vec::new()));
        Self(())
    }

    /// Every row recorded for `table` so far, as JSON, without its time.
    #[must_use]
    pub fn rows(&self, table: &str) -> Vec<serde_json::Value> {
        CAPTURED.with_borrow(|captured| {
            captured
                .iter()
                .flatten()
                .filter(|(name, _)| *name == table)
                .map(|(_, row)| {
                    let mut row = row.clone();
                    if let Some(row) = row.as_object_mut() {
                        row.remove("time");
                    }
                    row
                })
                .collect()
        })
    }

    /// Keeps `row` for `table` if this thread captures; whether it did. A
    /// signal is plain columns, so it always serializes.
    pub(super) fn take(table: &'static str, row: &impl Serialize) -> bool {
        CAPTURED.with_borrow_mut(|captured| {
            captured.as_mut().is_some_and(|rows| {
                rows.extend(serde_json::to_value(row).ok().map(|row| (table, row)));
                true
            })
        })
    }
}

impl Drop for AnalyticsCapture {
    /// Stops capturing.
    fn drop(&mut self) {
        CAPTURED.with_borrow_mut(|captured| *captured = None);
    }
}

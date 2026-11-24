//! The queue [`Signal::record`](super::Signal::record) fills, and the task that
//! sends it to ClickHouse in batches.

use super::Store;
use bytes::Bytes;
use chrono::Utc;
use serde::Serialize;
use sourceorigin_common::ErrorChain;
use std::{
    collections::HashMap,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::MissedTickBehavior,
};

/// Rows the queue holds before recording drops them: a few seconds of a busy
/// server's rows, so a slow ClickHouse costs rows, never memory or latency.
const QUEUE: usize = 10_000;
/// Rows of one table sent at once, at most.
const BATCH: usize = 5_000;
/// How often queued rows are sent, so ClickHouse gets few, large inserts.
const INTERVAL: Duration = Duration::from_secs(1);

/// The installed writer's queue, until the process exits.
static QUEUE_SENDER: OnceLock<mpsc::Sender<Message>> = OnceLock::new();
/// Rows dropped since the writer last reported them: queued while the queue was
/// full, or in a batch ClickHouse refused or never answered.
static DROPPED: AtomicU64 = AtomicU64::new(0);

/// What the queue carries.
enum Message {
    /// A row for a table, as a line of JSON.
    Row(&'static str, String),
    /// Send everything queued before it, then answer.
    Flush(oneshot::Sender<()>),
}

/// The row as stored: the signal's fields, after the time it was recorded.
#[derive(Serialize)]
struct Stamped<'a, S: ?Sized> {
    /// When it was recorded, as ClickHouse parses a `DateTime64(3)`.
    time: String,
    /// The signal's own columns.
    #[serde(flatten)]
    signal: &'a S,
}

/// Queues `signal` for `table`, unless a test captures it; see
/// [`Signal::record`](super::Signal::record).
pub(super) fn record<S: Serialize + ?Sized>(table: &'static str, signal: &S) {
    let row = Stamped {
        time: Utc::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
        signal,
    };
    #[cfg(any(test, feature = "test-util"))]
    if super::capture::AnalyticsCapture::take(table, &row) {
        return;
    }
    let Some(queue) = QUEUE_SENDER.get() else {
        return;
    };
    let Ok(line) = serde_json::to_string(&row) else {
        return;
    };
    if queue.try_send(Message::Row(table, line)).is_err() {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

/// The installed writer, which `Telemetry::shutdown` flushes.
pub(crate) struct Writer {
    /// Ends once its queue closes.
    task: JoinHandle<()>,
}

impl Writer {
    /// Starts sending queued rows to `store` on the current tokio runtime. Only
    /// the first writer of a process is installed: `None` for any later one.
    pub(crate) fn start(store: Store) -> Option<Self> {
        let (sender, queue) = mpsc::channel(QUEUE);
        QUEUE_SENDER.set(sender).ok()?;
        Some(Self {
            task: tokio::spawn(Self::run(store, queue)),
        })
    }

    /// Sends what was recorded so far. A row recorded once this is called may wait
    /// for the next interval.
    pub(crate) async fn flush(&self) {
        let (sent, done) = oneshot::channel();
        if let Some(queue) = QUEUE_SENDER.get()
            && queue.send(Message::Flush(sent)).await.is_ok()
        {
            done.await.ok();
        }
        if self.task.is_finished() {
            tracing::warn!("the analytics writer stopped");
        }
    }

    /// Batches rows by table, and sends each table's every [`INTERVAL`], or
    /// once it holds [`BATCH`] rows.
    async fn run(store: Store, mut queue: mpsc::Receiver<Message>) {
        let mut batches: HashMap<&'static str, Vec<String>> = HashMap::new();
        let mut interval = tokio::time::interval(INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                message = queue.recv() => match message {
                    Some(Message::Row(table, line)) => {
                        let batch = batches.entry(table).or_default();
                        batch.push(line);
                        if batch.len() >= BATCH {
                            let rows = std::mem::take(batch);
                            Self::send(&store, table, rows).await;
                        }
                    }
                    Some(Message::Flush(done)) => {
                        Self::send_all(&store, &mut batches).await;
                        let _ = done.send(());
                    }
                    None => return Self::send_all(&store, &mut batches).await,
                },
                _ = interval.tick() => Self::send_all(&store, &mut batches).await,
            }
        }
    }

    /// Sends every table's batch, and reports the rows dropped since the last
    /// report.
    async fn send_all(store: &Store, batches: &mut HashMap<&'static str, Vec<String>>) {
        for (table, rows) in batches.iter_mut() {
            if !rows.is_empty() {
                Self::send(store, table, std::mem::take(rows)).await;
            }
        }
        let dropped = DROPPED.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            tracing::warn!(analytics.dropped = dropped, "analytics rows dropped");
        }
    }

    /// Inserts `rows` into `table`, or drops and counts them.
    async fn send(store: &Store, table: &'static str, rows: Vec<String>) {
        let count = u64::try_from(rows.len()).unwrap_or(u64::MAX);
        let body = Bytes::from(rows.join("\n"));
        if let Err(error) = store.insert(table, body).await {
            DROPPED.fetch_add(count, Ordering::Relaxed);
            tracing::warn!(error = %ErrorChain(&error), "analytics insert failed");
        }
    }
}

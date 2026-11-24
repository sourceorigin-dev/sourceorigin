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
        LazyLock, Mutex, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::MissedTickBehavior,
};

/// Rows of one table the queue holds before recording drops that table's: a few
/// seconds of a busy server's requests, so a slow ClickHouse costs rows, never
/// memory or latency, and a flood of one table's rows, such as requests, never
/// costs another's.
pub(super) const QUEUE: usize = 5_000;
/// Rows of one table sent at once, at most: as many as it may queue.
const BATCH: usize = QUEUE;
/// How often queued rows are sent, so ClickHouse gets few, large inserts.
const INTERVAL: Duration = Duration::from_secs(1);
/// How often a product table's rows are sent: an insert's own time, which
/// ClickHouse logs, then counts an hour's people, not one sign-in.
const PRODUCT_INTERVAL: Duration = Duration::from_hours(1);

/// The installed writer's queue, until the process exits. [`QUEUED`] bounds it.
static QUEUE_SENDER: OnceLock<mpsc::UnboundedSender<Message>> = OnceLock::new();
/// Rows each table has in the queue.
pub(super) static QUEUED: LazyLock<Mutex<HashMap<&'static str, usize>>> =
    LazyLock::new(Mutex::default);
/// Rows dropped since the writer last reported them: recorded while their
/// table's queue was full, or in a batch ClickHouse refused or never answered.
pub(super) static DROPPED: AtomicU64 = AtomicU64::new(0);

/// What the queue carries.
enum Message {
    /// A row for a table, as a line of JSON, stamped as its table's are.
    Row(&'static str, Stamp, String),
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

/// How precisely a row's time is kept, and how soon it is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stamp {
    /// To the millisecond, as an operational row's.
    Millisecond,
    /// To the day, as a product row's, and sent every hour: a time precise
    /// enough to line it up with a sign-in's event would tell whose it is.
    Day,
}

impl Stamp {
    /// The current time, as precisely as this stamp keeps it, as ClickHouse
    /// parses a `DateTime64(3)`.
    pub(super) fn now(self) -> String {
        let format = match self {
            Self::Millisecond => "%Y-%m-%d %H:%M:%S%.3f",
            Self::Day => "%Y-%m-%d 00:00:00.000",
        };
        Utc::now().format(format).to_string()
    }
}

/// Queues `signal` for `table`, stamped to `stamp`, unless a test captures it;
/// see [`Signal::record`](super::Signal::record).
pub(super) fn record<S: Serialize + ?Sized>(table: &'static str, stamp: Stamp, signal: &S) {
    let row = Stamped {
        time: stamp.now(),
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
    let mut queued = QUEUED.lock().unwrap_or_else(PoisonError::into_inner);
    let count = queued.entry(table).or_default();
    if *count >= QUEUE || queue.send(Message::Row(table, stamp, line)).is_err() {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    *count += 1;
}

/// Each table's waiting rows, with how its rows are stamped.
type Batches = HashMap<&'static str, (Stamp, Vec<String>)>;

/// The installed writer, which `Telemetry::shutdown` flushes.
pub(crate) struct Writer {
    /// Ends once its queue closes.
    task: JoinHandle<()>,
}

impl Writer {
    /// Starts sending queued rows to `store` on the current tokio runtime. Only
    /// the first writer of a process is installed: `None` for any later one.
    pub(crate) fn start(store: Store) -> Option<Self> {
        let (sender, queue) = mpsc::unbounded_channel();
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
            && queue.send(Message::Flush(sent)).is_ok()
        {
            done.await.ok();
        }
        if self.task.is_finished() {
            tracing::warn!("the analytics writer stopped");
        }
    }

    /// Batches rows by table, and sends each table's every [`INTERVAL`], a
    /// product table's every [`PRODUCT_INTERVAL`], or once it holds [`BATCH`]
    /// rows.
    async fn run(store: Store, mut queue: mpsc::UnboundedReceiver<Message>) {
        let mut batches: Batches = HashMap::new();
        let mut interval = tokio::time::interval(INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut products_due = tokio::time::Instant::now() + PRODUCT_INTERVAL;
        loop {
            tokio::select! {
                message = queue.recv() => match message {
                    Some(Message::Row(table, stamp, line)) => {
                        let (_, batch) = batches.entry(table).or_insert((stamp, Vec::new()));
                        batch.push(line);
                        if batch.len() >= BATCH {
                            let rows = std::mem::take(batch);
                            Self::send(&store, table, rows).await;
                        }
                    }
                    Some(Message::Flush(done)) => {
                        Self::send_all(&store, &mut batches, true).await;
                        done.send(()).ok();
                    }
                    None => return Self::send_all(&store, &mut batches, true).await,
                },
                _ = interval.tick() => {
                    let products = tokio::time::Instant::now() >= products_due;
                    if products {
                        products_due += PRODUCT_INTERVAL;
                    }
                    Self::send_all(&store, &mut batches, products).await;
                }
            }
        }
    }

    /// Sends every table's batch, a product table's only with `products`, and
    /// reports the rows dropped since the last report.
    async fn send_all(store: &Store, batches: &mut Batches, products: bool) {
        for (table, (stamp, rows)) in batches.iter_mut() {
            if !rows.is_empty() && (products || *stamp != Stamp::Day) {
                Self::send(store, table, std::mem::take(rows)).await;
            }
        }
        let dropped = DROPPED.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            tracing::warn!(analytics.dropped = dropped, "analytics rows dropped");
        }
    }

    /// Inserts `rows` into `table`, or drops and counts them. Either way they
    /// leave the queue.
    async fn send(store: &Store, table: &'static str, rows: Vec<String>) {
        if let Some(queued) = QUEUED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(table)
        {
            *queued = queued.saturating_sub(rows.len());
        }
        let count = u64::try_from(rows.len()).unwrap_or(u64::MAX);
        let body = Bytes::from(rows.join("\n"));
        if let Err(error) = store.insert(table, body).await {
            DROPPED.fetch_add(count, Ordering::Relaxed);
            tracing::warn!(error = %ErrorChain(&error), "analytics insert failed");
        }
    }
}

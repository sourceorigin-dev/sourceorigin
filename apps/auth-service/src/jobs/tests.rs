//! Job runner failure isolation and exclusivity across runners, and the alert
//! of failing purges.
use super::*;
use crate::tests::Replicas;

/// Panics on its first run and counts every run.
struct PanicsOnce(watch::Sender<u32>);

impl ExclusiveJob for PanicsOnce {
    const NAME: &'static str = "panics-once";
    const LOCK: LockKey = LockKey::Test;
    const EVERY: Duration = Duration::from_millis(20);
    type Error = DatabaseError;

    fn run(
        &self,
        _: &mut ExclusiveConnection,
        _: &Shutdown,
    ) -> impl Future<Output = Result<(), DatabaseError>> + Send {
        self.0.send_modify(|runs| *runs += 1);
        assert_ne!(*self.0.borrow(), 1, "first run panics");
        std::future::ready(Ok(()))
    }
}

/// A run that panics ends only that run: its lock is freed, the job runs again,
/// and shutdown reports nothing because the job's loop survived.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_panicking_run_releases_its_lock_and_the_job_runs_again() {
    let runs = watch::Sender::new(0);
    let mut counted = runs.subscribe();
    let mut jobs = Jobs::new();
    jobs.exclusive(PanicsOnce(runs), Replicas::database().await);
    tokio::time::timeout(Duration::from_secs(5), counted.wait_for(|&runs| runs >= 2))
        .await
        .expect("the job runs again after panicking")
        .expect("the job lives until shutdown");
    jobs.shutdown().await.expect("no job loop panicked");
}

/// A run of [`Held`] by the runner it names starting, finishing or being
/// cancelled, or the runner finding the other one running it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Started(usize),
    Finished(usize),
    Cancelled(usize),
    Excluded(usize),
}

/// Every [`Event`] of the runners sharing it, in order.
#[derive(Default)]
struct Log(watch::Sender<Vec<Event>>);

impl Log {
    /// Appends `event`, and returns its position.
    fn push(&self, event: Event) -> usize {
        let mut position = 0;
        self.0.send_modify(|events| {
            position = events.len();
            events.push(event);
        });
        position
    }

    /// The runs' events so far, without the exclusions.
    fn runs(&self) -> Vec<Event> {
        Self::runs_of(&self.0.borrow())
    }

    /// The runs' events among `events`.
    fn runs_of(events: &[Event]) -> Vec<Event> {
        events
            .iter()
            .filter(|event| !matches!(event, Event::Excluded(_)))
            .cloned()
            .collect()
    }

    /// Waits until `found` returns something for the events so far.
    async fn until<T>(&self, found: impl Fn(&[Event]) -> Option<T>) -> T {
        let mut events = self.0.subscribe();
        let events = events
            .wait_for(|events| found(events).is_some())
            .await
            .expect("the log outlives its waiters");
        found(&events).expect("found")
    }

    /// [`Self::until`], failing after five seconds.
    async fn wait_for<T>(&self, found: impl Fn(&[Event]) -> Option<T>) -> T {
        tokio::time::timeout(Duration::from_secs(5), self.until(found))
            .await
            .unwrap_or_else(|_| panic!("the awaited events never happened: {:?}", *self.0.borrow()))
    }
}

/// Holds its lock until the other runner finds it held, or shutdown, logging
/// when each run of `runner` starts and how it ends.
struct Held {
    runner: usize,
    log: Arc<Log>,
}

/// Logs how a [`Held`] run ended when dropped, so a cancelled run is logged
/// too. The run drops it before its connection, so before the lock is freed.
struct Running<'a> {
    job: &'a Held,
    finished: bool,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        let runner = self.job.runner;
        self.job.log.push(if self.finished {
            Event::Finished(runner)
        } else {
            Event::Cancelled(runner)
        });
    }
}

impl ExclusiveJob for Held {
    const NAME: &'static str = "held";
    const LOCK: LockKey = LockKey::ExclusivityTest;
    const EVERY: Duration = Duration::from_millis(20);
    type Error = DatabaseError;

    async fn run(
        &self,
        _: &mut ExclusiveConnection,
        shutdown: &Shutdown,
    ) -> Result<(), DatabaseError> {
        let started = self.log.push(Event::Started(self.runner));
        let mut running = Running {
            job: self,
            finished: false,
        };
        let other = Event::Excluded(1 - self.runner);
        let excluded = self
            .log
            .until(|events| events[started..].contains(&other).then_some(()));
        shutdown.unless(excluded).await;
        running.finished = true;
        Ok(())
    }

    fn excluded(&self) {
        self.log.push(Event::Excluded(self.runner));
    }
}

/// Two runners, as two replicas, never run the same exclusive job at once: each
/// run lasts until the other runner finds it running. Dropping the one mid-run
/// cancels its run, freeing the lock for the other's next tick.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn runners_never_overlap_and_a_dropped_holder_hands_over() {
    let database = Replicas::database().await;
    let log = Arc::new(Log::default());
    let mut runners = [0, 1].map(|runner| {
        let mut jobs = Jobs::new();
        let log = Arc::clone(&log);
        jobs.exclusive(Held { runner, log }, database.clone());
        Some(jobs)
    });
    // A few runs in, cancel whichever runner is mid-run.
    let holder = log
        .wait_for(|events| match Log::runs_of(events).as_slice() {
            runs @ [.., Event::Started(holder)] if runs.len() > 6 => Some(*holder),
            _ => None,
        })
        .await;
    drop(runners[holder].take());
    let other = 1 - holder;
    log.wait_for(|events| {
        Log::runs_of(events)
            .windows(2)
            .any(|pair| {
                matches!(
                    pair,
                    [Event::Cancelled(cancelled), Event::Started(started)]
                        if *cancelled == holder && *started == other
                )
            })
            .then_some(())
    })
    .await;
    runners[other]
        .take()
        .expect("the other runner")
        .shutdown()
        .await
        .expect("no job loop panicked");
    let events = log.runs();
    assert!(
        events.chunks(2).all(|run| matches!(
            run,
            [Event::Started(started), Event::Finished(ended) | Event::Cancelled(ended)]
                if started == ended
        )),
        "runs overlapped: {events:?}"
    );
}

/// A purge run raises the alert of accounts that keep failing to be purged, and
/// a run with none clears it. The database is its own, since the count is every
/// account's.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn failing_purges_raise_their_alert_until_none_fail() {
    let harness = crate::admin::tests::Harness::isolated().await;
    let database = &harness.database;
    let user_id = harness.account().await.user_id;
    // Failing, and waiting a day for its next attempt, so the run purges nothing.
    let mut deletions = database.deletions();
    deletions
        .schedule(user_id, Utc::now() - chrono::Duration::days(1))
        .await
        .expect("schedule");
    for _ in 0..AccountPurge::FAILING_ATTEMPTS {
        let day = chrono::Duration::days(1);
        deletions
            .record_failure(user_id, "boom", day, day)
            .await
            .expect("failure");
    }
    let mut lock = database
        .try_exclusive(LockKey::AccountPurge)
        .await
        .expect("lock")
        .expect("no other test purges");
    let shutdown = Shutdown(watch::Sender::new(false).subscribe());
    let current = async || current_alert(database, AlertKind::PurgesFailing).await;
    AccountPurge.run(&mut lock, &shutdown).await.expect("run");
    assert_eq!(current().await, Some(Alert::PurgesFailing {}));
    assert!(database.deletions().cancel(user_id).await.expect("cancel"));
    AccountPurge.run(&mut lock, &shutdown).await.expect("run");
    assert_eq!(current().await, None);
    lock.release().await.expect("release");
}

/// The third cleanup pass in a row that runs out of time raises the alert of
/// cleanup falling behind, the fourth marks it seen without raising another,
/// and a pass that deletes everything due clears it.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn cleanup_that_keeps_falling_behind_raises_its_alert() {
    let database = crate::admin::tests::Harness::isolated().await.database;
    let mut lock = database.exclusive(LockKey::Cleanup).await.expect("lock");
    let shutdown = Shutdown(watch::Sender::new(false).subscribe());
    // Out of time as soon as it deletes anything.
    let cleanup = Cleanup {
        budget: Duration::ZERO,
        overruns: Streak::new(Cleanup::BEHIND_AFTER),
        failures: Streak::new(Cleanup::FAILING_AFTER),
    };
    let current = async || {
        let mut tx = database.begin().await.expect("transaction");
        let current = tx
            .alerts()
            .current(AlertKind::CleanupBehind)
            .await
            .expect("current");
        tx.commit().await.expect("commit");
        current
    };
    let mut raised = None;
    for pass in 1..=Cleanup::BEHIND_AFTER + 1 {
        // An expired invitation, for the pass to delete.
        let digest = UserId::new().as_uuid().as_u128().to_be_bytes().repeat(2);
        let expired = Utc::now() - chrono::Duration::days(1);
        database
            .security()
            .create_invitation(&sourceorigin_repository::InsertInvitation {
                id: sourceorigin_common::TokenId::new(),
                secret_digest: &digest,
                expires_at: expired,
                email: None,
            })
            .await
            .expect("invitation");
        cleanup.run(&mut lock, &shutdown).await.expect("run");
        let found = current().await;
        match pass.cmp(&Cleanup::BEHIND_AFTER) {
            std::cmp::Ordering::Less => assert_eq!(found, None, "after pass {pass}"),
            std::cmp::Ordering::Equal => {
                let found = found.expect("raised");
                assert_eq!(found.alert, Some(Alert::CleanupBehind {}));
                raised = Some(found);
            }
            std::cmp::Ordering::Greater => {
                let (found, raised) = (found.expect("current"), raised.take().expect("raised"));
                assert_eq!(found.id, raised.id, "not raised again");
                assert!(found.seen_at > raised.seen_at, "seen again");
            }
        }
    }
    cleanup.run(&mut lock, &shutdown).await.expect("run");
    assert_eq!(current().await, None);
    lock.release().await.expect("release");
}

/// A streak raises its alert once it is long enough, and only once, then marks
/// it seen. Once the alert cleared meanwhile, the run that found it so starts a
/// new streak, which takes as many runs to raise it again; so does a reset.
#[test]
fn a_streak_raises_its_alert_once() {
    let streak = Streak::new(3);
    assert_eq!(streak.extend(), None);
    assert_eq!(streak.extend(), None);
    assert_eq!(
        streak.extend(),
        Some(Due::Raise),
        "the third in a row raises it"
    );
    assert_eq!(streak.extend(), Some(Due::Raise), "until raising it worked");
    streak.settle(true);
    assert_eq!(streak.extend(), Some(Due::Seen), "then marks it seen");
    streak.settle(false);
    assert_eq!(
        streak.extend(),
        None,
        "the run that found it cleared counts"
    );
    assert_eq!(streak.extend(), Some(Due::Raise), "as the first of 3");
    streak.reset();
    assert_eq!(streak.extend(), None);
    assert_eq!(streak.extend(), None);
    assert_eq!(
        streak.extend(),
        Some(Due::Raise),
        "a new streak raises it again"
    );
}

/// The third cleanup pass in a row that fails raises the alert of cleanup
/// failing, and a pass that works clears it.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn cleanup_that_keeps_failing_raises_its_alert() {
    let database = crate::admin::tests::Harness::isolated().await.database;
    let mut lock = database.exclusive(LockKey::Cleanup).await.expect("lock");
    let shutdown = Shutdown(watch::Sender::new(false).subscribe());
    let cleanup = Cleanup::new();
    let current = async || current_alert(&database, AlertKind::CleanupFailing).await;
    for pass in 1..=Cleanup::FAILING_AFTER {
        let failed = Err(DatabaseError::NotFound);
        assert!(cleanup.ended(&mut lock, failed).await.is_err());
        let raised = (pass == Cleanup::FAILING_AFTER).then_some(Alert::CleanupFailing {});
        assert_eq!(current().await, raised, "after pass {pass}");
    }
    cleanup.run(&mut lock, &shutdown).await.expect("run");
    assert_eq!(current().await, None);
    lock.release().await.expect("release");
}

/// What the current alert of `kind` on `database` says, if one is current.
async fn current_alert(database: &Database, kind: AlertKind) -> Option<Alert> {
    let mut tx = database.begin().await.expect("transaction");
    let current = tx.alerts().current(kind).await.expect("current");
    tx.commit().await.expect("commit");
    current.and_then(|current| current.alert)
}

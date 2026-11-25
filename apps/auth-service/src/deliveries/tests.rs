//! Retry policy, the HTTP client's safety checks and the worker loop.
use super::outbound::{OutboundClient, Reachable};
use super::*;
use crate::tests::{Replicas, TlsListener};
use axum::{
    Router,
    http::{HeaderMap, HeaderValue, Method, header::RETRY_AFTER},
    response::IntoResponse,
    routing::get,
};
use sourceorigin_common::DeliveryId;
use sourceorigin_repository::{DeliveryStatus, DeliverySummary, NewDelivery};
use std::{marker::PhantomData, net::IpAddr};
use tokio::sync::{Notify, mpsc, watch};

/// A retryable failure without `Retry-After`.
const UNAVAILABLE: DeliveryError = DeliveryError::Retryable {
    failure: Failure::Status(StatusCode::SERVICE_UNAVAILABLE),
    retry_after: None,
};

/// The outcome of `error` on a delivery's `attempts`th attempt, an hour after its
/// creation, for a channel retrying for a day.
fn outcome(error: DeliveryError, attempts: u64, jitter: f64) -> DeliveryOutcome {
    let created_at = DateTime::UNIX_EPOCH;
    let now = created_at + TimeDelta::hours(1);
    error.outcome(attempts, created_at + TimeDelta::days(1), now, jitter)
}

/// When `outcome` retries, measured from the attempt.
fn delay(outcome: &DeliveryOutcome) -> Option<TimeDelta> {
    match outcome {
        DeliveryOutcome::Retry { at, .. } => Some(*at - DateTime::UNIX_EPOCH - TimeDelta::hours(1)),
        DeliveryOutcome::Succeeded | DeliveryOutcome::Failed { .. } => None,
    }
}

/// Retries wait a random time up to a bound doubling from 10 s to an hour, or
/// longer if `Retry-After` asks; a shorter `Retry-After` does not skip the backoff. Permanent errors, and retries past the channel's
/// maximum age, fail at once.
#[test]
fn retries_back_off_and_honor_retry_after_until_the_maximum_age() {
    let halfway = |attempts| delay(&outcome(UNAVAILABLE, attempts, 0.5));
    assert_eq!(halfway(1), Some(TimeDelta::seconds(5)));
    assert_eq!(halfway(3), Some(TimeDelta::seconds(20)));
    assert_eq!(halfway(10), Some(TimeDelta::minutes(30)));
    assert_eq!(halfway(u64::MAX), Some(TimeDelta::minutes(30)));
    assert_eq!(
        outcome(UNAVAILABLE, 3, 0.0),
        DeliveryOutcome::Retry {
            at: DateTime::UNIX_EPOCH + TimeDelta::hours(1),
            error: "HTTP 503".into(),
        }
    );

    let asked = |seconds| DeliveryError::Retryable {
        failure: Failure::Timeout,
        retry_after: Some(Duration::from_secs(seconds)),
    };
    assert_eq!(
        delay(&outcome(asked(120), 1, 0.5)),
        Some(TimeDelta::seconds(120))
    );
    assert_eq!(
        delay(&outcome(asked(0), 3, 0.5)),
        Some(TimeDelta::seconds(20))
    );
    assert_eq!(
        outcome(asked(24 * 3600), 1, 0.5),
        DeliveryOutcome::Failed {
            error: "timed out".into()
        },
        "past the maximum age"
    );
    assert_eq!(
        outcome(DeliveryError::Permanent(Failure::Redirect), 1, 0.5),
        DeliveryOutcome::Failed {
            error: "redirect refused".into()
        }
    );
    let jitter = DeliveryError::jitter();
    assert!((0.0..1.0).contains(&jitter), "{jitter}");
}

/// Only public addresses are reachable, by the shared check
/// (`sourceorigin_common::net`, whose tests list the ranges), and loopback too in
/// tests.
#[test]
fn only_public_addresses_are_reachable() {
    let address = |text: &str| -> IpAddr { text.parse().expect("address") };
    assert!(Reachable::Public.allows(address("93.184.215.14")));
    assert!(!Reachable::Public.allows(address("100.64.0.1")));
    assert!(Reachable::PublicOrLoopback.allows(address("::1")));
}

/// Only 408, 429 and 5xx are retried, after `Retry-After` in seconds or as a date
/// in any HTTP-date format.
#[test]
fn statuses_classify_and_retry_after_parses() {
    let now = DateTime::parse_from_rfc2822("Wed, 21 Oct 2026 07:28:00 GMT")
        .expect("date")
        .with_timezone(&Utc);
    let check = |status: StatusCode, retry_after: Option<&'static str>| {
        let mut headers = HeaderMap::new();
        if let Some(value) = retry_after {
            headers.insert(RETRY_AFTER, HeaderValue::from_static(value));
        }
        OutboundClient::check_status(status, &headers, now)
    };
    let retryable = |status, seconds: Option<u64>| {
        Err(DeliveryError::Retryable {
            failure: Failure::Status(status),
            retry_after: seconds.map(Duration::from_secs),
        })
    };
    assert_eq!(check(StatusCode::NO_CONTENT, None), Ok(()));
    assert_eq!(
        check(StatusCode::TOO_MANY_REQUESTS, Some("120")),
        retryable(StatusCode::TOO_MANY_REQUESTS, Some(120))
    );
    assert_eq!(
        check(
            StatusCode::SERVICE_UNAVAILABLE,
            Some("Wed, 21 Oct 2026 07:30:00 GMT")
        ),
        retryable(StatusCode::SERVICE_UNAVAILABLE, Some(120))
    );
    for obsolete in [
        "Wednesday, 21-Oct-26 07:30:00 GMT",
        "Wed Oct 21 07:30:00 2026",
    ] {
        assert_eq!(
            check(StatusCode::SERVICE_UNAVAILABLE, Some(obsolete)),
            retryable(StatusCode::SERVICE_UNAVAILABLE, Some(120)),
            "{obsolete}"
        );
    }
    assert_eq!(
        check(StatusCode::BAD_GATEWAY, Some("soon")),
        retryable(StatusCode::BAD_GATEWAY, None)
    );
    assert_eq!(
        check(StatusCode::REQUEST_TIMEOUT, None),
        retryable(StatusCode::REQUEST_TIMEOUT, None)
    );
    assert_eq!(
        check(StatusCode::BAD_REQUEST, Some("120")),
        Err(DeliveryError::Permanent(Failure::Status(
            StatusCode::BAD_REQUEST
        )))
    );
}

/// Destinations that are not HTTPS, or are or resolve to a loopback address, are
/// refused before anything is sent.
#[tokio::test]
async fn non_public_and_plain_http_destinations_are_refused() {
    let client = OutboundClient::new().expect("client");
    for (destination, failure) in [
        ("https://localhost:9/", Failure::ForbiddenDestination),
        ("https://127.0.0.1:9/", Failure::ForbiddenDestination),
        (
            "https://[::ffff:192.168.0.1]/",
            Failure::ForbiddenDestination,
        ),
        ("http://example.com/", Failure::InvalidDestination),
        ("not a URL", Failure::InvalidDestination),
    ] {
        let result = client
            .send(Method::POST, destination, |request| request)
            .await;
        assert_eq!(
            result.err(),
            Some(DeliveryError::Permanent(failure)),
            "{destination}"
        );
    }
}

/// A successful status is accepted before any of the body arrives, so a receiver
/// that stalls after its headers still counts as reached. A redirect is refused
/// rather than followed, a busy receiver's `Retry-After` comes back with its
/// retryable status, and a fetched file is cut at its limit.
#[tokio::test]
async fn responses_are_classified_by_status_alone() {
    let tcp = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind receiver");
    let origin = format!(
        "https://localhost:{}",
        tcp.local_addr().expect("address").port()
    );
    let receiver = Router::new()
        .route(
            "/stalled",
            get(|| async {
                axum::body::Body::from_stream(tokio_stream::pending::<
                    Result<Vec<u8>, std::convert::Infallible>,
                >())
            }),
        )
        .route(
            "/moved",
            get(|| async { axum::response::Redirect::temporary("https://example.com/") }),
        )
        .route(
            "/busy",
            get(|| async {
                (StatusCode::SERVICE_UNAVAILABLE, [(RETRY_AFTER, "30")]).into_response()
            }),
        )
        .route("/long", get(|| async { vec![b'a'; 100 * 1024] }));
    let client = OutboundClient::local(TlsListener::serve(tcp, receiver));
    let send = async |path: &str| {
        client
            .send(Method::GET, &format!("{origin}{path}"), |request| request)
            .await
    };
    let stalled = tokio::time::timeout(SEND_TIMEOUT / 2, send("/stalled"));
    assert_eq!(stalled.await, Ok(Ok(())));
    assert_eq!(
        send("/moved").await,
        Err(DeliveryError::Permanent(Failure::Redirect))
    );
    assert_eq!(
        send("/busy").await,
        Err(DeliveryError::Retryable {
            failure: Failure::Status(StatusCode::SERVICE_UNAVAILABLE),
            retry_after: Some(Duration::from_secs(30)),
        })
    );
    let long = client
        .get(&format!("{origin}/long"), "*/*", 64 * 1024)
        .await
        .map(|body| body.len());
    assert_eq!(long, Ok(64 * 1024));
}

sourceorigin_repository::test_channels! {
    /// Retryable and permanent failures.
    Failing = "test_worker": Behavior => ();
    /// An operator retry after a permanent failure.
    Resent = "test_worker_resent": Behavior => ();
    /// What an attempt and an idle worker export.
    Traced = "test_worker_traced": Behavior => ();
    /// A replica killed, or stalled, while it sends.
    Killed = "test_worker_killed": Behavior => ();
}

/// How [`Recorder`] answers a delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Behavior {
    /// Always retryable, asking to be retried at once.
    Unavailable,
    /// Permanently refused.
    Refused,
    /// Permanently refused on the first attempt, accepted after.
    RefusedOnce,
    /// Accepted.
    Accepted,
}

/// What a channel's workers did: the delivery ID each attempt carried, in
/// order, and how many of their polls found nothing due.
#[derive(Clone, Debug, Default)]
struct Seen {
    attempts: Vec<DeliveryId>,
    idle: usize,
}

impl Seen {
    /// How many attempts carried `id`.
    fn count(&self, id: DeliveryId) -> usize {
        self.attempts.iter().filter(|&&seen| seen == id).count()
    }
}

/// Sends channel `C`'s deliveries, publishing what its workers do as each
/// attempt starts and each poll finds nothing.
struct Recorder<C>(watch::Sender<Seen>, PhantomData<C>);

impl<C: Channel<Recipient = Behavior, Payload = ()>> Sender for Recorder<C> {
    type Channel = C;
    const MAX_AGE: Duration = Duration::from_hours(1);

    fn send(
        &self,
        delivery: &Delivery<C>,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        let mut first = false;
        self.0.send_modify(|seen| {
            first = !seen.attempts.contains(&delivery.id);
            seen.attempts.push(delivery.id);
        });
        let refused = DeliveryError::Permanent(Failure::Status(StatusCode::FORBIDDEN));
        std::future::ready(match delivery.recipient {
            Behavior::Unavailable => Err(DeliveryError::Retryable {
                failure: Failure::Connection,
                retry_after: Some(Duration::ZERO),
            }),
            Behavior::Refused => Err(refused),
            Behavior::RefusedOnce if first => Err(refused),
            Behavior::RefusedOnce | Behavior::Accepted => Ok(()),
        })
    }

    fn idle(&self) {
        self.0.send_modify(|seen| seen.idle += 1);
    }
}

/// A database with no pending delivery left on `C` by an earlier run, workers
/// sending `C`, and what they do.
async fn workers<C: Channel<Recipient = Behavior, Payload = ()>>()
-> (Database, Jobs, watch::Sender<Seen>) {
    let database = cleared::<C>().await;
    let seen = watch::Sender::default();
    let jobs = Jobs::replica(Recorder::<C>(seen.clone(), PhantomData)).await;
    (database, jobs, seen)
}

/// A database with no pending delivery left on `C` by an earlier run.
async fn cleared<C: Channel>() -> Database {
    let database = Replicas::database().await;
    for summary in database
        .deliveries()
        .list(Some(DeliveryStatus::Pending), Some(C::NAME), 1000)
        .await
        .expect("list")
    {
        database
            .deliveries()
            .cancel(summary.id)
            .await
            .expect("clear");
    }
    database
}

impl Jobs {
    /// One replica's workers sending with `sender`, on a pool only they hold:
    /// dropping them aborts their tasks and closes its connections, as killing the
    /// replica's process would.
    async fn replica<S: Sender>(sender: S) -> Self {
        let mut jobs = Self::new();
        jobs.deliveries(sender, &Replicas::database().await);
        jobs
    }
}

/// Queues a delivery on `C` that the receiver answers with `behavior`.
async fn queue<C: Channel<Recipient = Behavior, Payload = ()>>(
    database: &Database,
    behavior: Behavior,
) -> DeliveryId {
    let delivery = NewDelivery::<C> {
        destination: "https://receiver.example".into(),
        recipient: behavior,
        payload: (),
    };
    database
        .deliveries()
        .insert(None, delivery)
        .await
        .expect("insert")
}

/// Waits up to 10 s until what the workers did satisfies `done`, and returns
/// it. An attempt shows as it starts: shutting the workers down finishes it and
/// records its outcome.
async fn wait_for(seen: &watch::Sender<Seen>, done: impl FnMut(&Seen) -> bool) -> Seen {
    let mut receiver = seen.subscribe();
    tokio::time::timeout(Duration::from_secs(10), receiver.wait_for(done))
        .await
        .expect("the workers do what the test awaits")
        .expect("the test holds a sender")
        .clone()
}

/// Delivery `id` of channel `C`, as an operator lists it.
async fn listed<C: Channel>(database: &Database, id: DeliveryId) -> DeliverySummary {
    database
        .deliveries()
        .list(None, Some(C::NAME), 1000)
        .await
        .expect("list")
        .into_iter()
        .find(|delivery| delivery.id == id)
        .expect("the delivery")
}

/// Workers record a retryable failure as a retry, with every attempt carrying
/// the delivery's ID, and fail a permanent one after one attempt.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn workers_retry_retryable_errors_and_fail_permanent_ones() {
    let (database, jobs, seen) = workers::<Failing>().await;
    let unavailable = queue::<Failing>(&database, Behavior::Unavailable).await;
    let refused = queue::<Failing>(&database, Behavior::Refused).await;
    wait_for(&seen, |seen| {
        seen.count(unavailable) > 0 && seen.count(refused) > 0
    })
    .await;
    jobs.shutdown().await.expect("no worker panicked");
    let unavailable_summary = listed::<Failing>(&database, unavailable).await;
    let refused_summary = listed::<Failing>(&database, refused).await;

    assert_eq!(unavailable_summary.status, DeliveryStatus::Pending);
    assert_eq!(
        unavailable_summary.last_error.as_deref(),
        Some("connection failed")
    );
    assert!(
        unavailable_summary.next_attempt_at
            <= Utc::now() + TimeDelta::from_std(MAX_BACKOFF).expect("backoff"),
        "retried within its backoff"
    );
    assert_eq!(
        (refused_summary.status, refused_summary.attempts),
        (DeliveryStatus::Failed, 1)
    );
    assert_eq!(refused_summary.last_error.as_deref(), Some("HTTP 403"));
    let seen = seen.borrow();
    assert_eq!(
        (seen.count(refused), seen.attempts.len()),
        (1, seen.count(unavailable) + 1),
        "every attempt carries its delivery's ID"
    );
}

/// Each attempt is a trace of its own, holding its queries, and an idle worker's
/// polls export nothing.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn each_attempt_is_a_trace_and_idle_polls_export_nothing() {
    let capture = sourceorigin_telemetry::Telemetry::capture(crate::TELEMETRY_FILTER);
    let (database, jobs, seen) = workers::<Traced>().await;
    let id = queue::<Traced>(&database, Behavior::Refused).await;
    let attempted = wait_for(&seen, |seen| seen.attempts.contains(&id)).await;
    wait_for(&seen, |seen| seen.idle > attempted.idle).await;
    jobs.shutdown().await.expect("no worker panicked");
    assert_eq!(
        listed::<Traced>(&database, id).await.status,
        DeliveryStatus::Failed
    );

    let spans = capture.spans();
    let names: Vec<_> = spans.iter().map(|span| span.name.as_ref()).collect();
    let attempt = spans
        .iter()
        .find(|span| span.name == Traced::NAME)
        .unwrap_or_else(|| panic!("the attempt's span: {names:?}"));
    let trace = attempt.span_context.trace_id();
    assert!(
        spans
            .iter()
            .all(|span| span.span_context.trace_id() == trace),
        "only the attempt is traced: {names:?}"
    );
    assert!(spans.len() > 1, "the attempt's queries are in its trace");
}

/// A delivery sent again after an operator retry carries the same ID, which
/// receivers deduplicate on as its `jti` or `Message-ID`.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_resent_delivery_carries_the_same_id() {
    let (database, jobs, seen) = workers::<Resent>().await;
    let id = queue::<Resent>(&database, Behavior::RefusedOnce).await;
    wait_for(&seen, |seen| !seen.attempts.is_empty()).await;
    jobs.shutdown().await.expect("no worker panicked");
    assert_eq!(
        listed::<Resent>(&database, id).await.status,
        DeliveryStatus::Failed
    );
    database.deliveries().retry(id).await.expect("retry");
    let jobs = Jobs::replica(Recorder::<Resent>(seen.clone(), PhantomData)).await;
    wait_for(&seen, |seen| seen.attempts.len() > 1).await;
    jobs.shutdown().await.expect("no worker panicked");

    let resent = listed::<Resent>(&database, id).await;
    assert_eq!(
        (resent.status, resent.attempts),
        (DeliveryStatus::Succeeded, 2)
    );
    assert_eq!(
        seen.borrow().attempts,
        [id, id],
        "both sends carry the delivery's ID"
    );
}

/// Replica A's sender: reports the ID its attempt carries, hangs until `release`,
/// then refuses the delivery. Its lease is short, so a stalled attempt outlives it.
struct Stalled {
    /// Receives each attempt's delivery ID as its send starts.
    entered: mpsc::UnboundedSender<DeliveryId>,
    /// Ends the hanging sends.
    release: Arc<Notify>,
}

impl Sender for Stalled {
    type Channel = Killed;
    const MAX_AGE: Duration = Duration::from_hours(1);
    const LEASE: Duration = Duration::from_secs(1);

    async fn send(&self, delivery: &Delivery<Killed>) -> Result<(), DeliveryError> {
        self.entered.send(delivery.id).expect("the test listens");
        self.release.notified().await;
        Err(DeliveryError::Permanent(Failure::Status(
            StatusCode::FORBIDDEN,
        )))
    }
}

/// A delivery whose replica is killed mid-send, or stalls there, is sent by the
/// other replica, with the same ID, once its lease expires. A stalled replica's
/// late outcome, here a refusal, is discarded.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_delivery_survives_its_replica_dying_mid_send() {
    for killed in [true, false] {
        let database = cleared::<Killed>().await;
        let (entered, mut sends) = mpsc::unbounded_channel();
        let release = Arc::new(Notify::new());
        let a = Jobs::replica(Stalled {
            entered,
            release: Arc::clone(&release),
        })
        .await;
        let id = queue::<Killed>(&database, Behavior::Accepted).await;
        let first = tokio::time::timeout(Duration::from_secs(10), sends.recv()).await;
        assert_eq!(first, Ok(Some(id)), "A sends the delivery");
        let seen = watch::Sender::default();
        let b = Jobs::replica(Recorder::<Killed>(seen.clone(), PhantomData)).await;
        let a = if killed {
            drop(a);
            None
        } else {
            Some(a)
        };

        wait_for(&seen, |seen| !seen.attempts.is_empty()).await;
        b.shutdown().await.expect("no worker panicked");
        if let Some(a) = a {
            release.notify_waiters();
            a.shutdown().await.expect("A's late attempt ends");
        }

        let delivered = listed::<Killed>(&database, id).await;
        let finished = delivered.finished_at.expect("finished") - delivered.created_at;
        assert!(
            finished >= TimeDelta::from_std(Stalled::LEASE).expect("lease"),
            "B waits for A's lease: {finished}, killed: {killed}"
        );
        assert_eq!(seen.borrow().attempts, [id], "B sends the same ID");
        assert_eq!(
            (delivered.status, delivered.last_error),
            (DeliveryStatus::Succeeded, None),
            "A's late refusal is discarded, killed: {killed}"
        );
    }
}

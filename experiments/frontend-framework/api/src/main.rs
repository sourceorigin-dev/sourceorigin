//! The stub API both candidates talk to: one change with a large diff, its
//! comment threads, reviewed marks, live events, and an account to edit.
//!
//! Everything is in memory and restarts clean. `stub-api --openapi` prints the
//! OpenAPI document the TypeScript client is generated from.

mod patch;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    convert::Infallible,
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use patch::{FilePatch, FileStatus, Side};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};
use tower_http::cors::CorsLayer;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};

/// The only change the stub holds.
const CHANGE: &str = "1";
/// Who the caller is, since the stub has no sign-in.
const ME: &str = "mira";

/// One file of a change, without its diff.
#[derive(Clone, Serialize, ToSchema)]
struct FileSummary {
    path: String,
    /// The path before a rename.
    old_path: Option<String>,
    status: FileStatus,
    additions: u32,
    deletions: u32,
    /// Whether the caller marked this file as reviewed.
    reviewed: bool,
    /// Counts up each time the file's diff changes while the change is open.
    revision: u32,
}

/// A change under review.
#[derive(Serialize, ToSchema)]
struct ChangeDetail {
    id: String,
    title: String,
    description: String,
    author: String,
    base: String,
    head: String,
    additions: u32,
    deletions: u32,
    files: Vec<FileSummary>,
}

/// One comment of a thread.
#[derive(Clone, Serialize, ToSchema)]
struct Comment {
    id: u32,
    author: String,
    body: String,
    /// Seconds since the Unix epoch.
    created_at: u64,
}

/// A comment thread on one line of one file.
#[derive(Clone, Serialize, ToSchema)]
struct Thread {
    id: u32,
    path: String,
    side: Side,
    line: u32,
    comments: Vec<Comment>,
}

/// A new thread, with its first comment.
#[derive(Deserialize, ToSchema)]
struct NewThread {
    path: String,
    side: Side,
    line: u32,
    body: String,
}

/// A reply to a thread.
#[derive(Deserialize, ToSchema)]
struct NewComment {
    body: String,
}

/// Whether the caller has reviewed a file.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
struct ReviewedMark {
    path: String,
    reviewed: bool,
}

/// What happened to a change while a page had it open.
#[derive(Clone, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ChangeEvent {
    /// Someone started a thread.
    ThreadCreated { thread: Thread },
    /// Someone replied to a thread.
    CommentAdded { thread_id: u32, comment: Comment },
    /// The author pushed a new version of a file: fetch its patch again.
    FileRevised { file: FileSummary },
}

/// Which update the stub should pretend someone else made.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum SimulateKind {
    Thread,
    Revision,
}

/// Asks the stub to make an update arrive, as another person would.
#[derive(Deserialize, ToSchema)]
struct Simulate {
    kind: SimulateKind,
    /// The file to act on. Without it the stub picks one.
    path: Option<String>,
}

/// The caller's account settings.
#[derive(Clone, Serialize, ToSchema)]
struct Account {
    display_name: String,
    email: String,
    bio: String,
    notify_on_review: bool,
    /// How many other browsers are signed in.
    other_sessions: u32,
}

/// The editable part of an account.
#[derive(Deserialize, ToSchema)]
struct AccountUpdate {
    display_name: String,
    email: String,
    bio: String,
    notify_on_review: bool,
}

/// Why a request failed. `fields` names each invalid field of a form.
#[derive(Serialize, ToSchema)]
struct Problem {
    message: String,
    fields: BTreeMap<String, String>,
}

impl Problem {
    /// A failure that is not about a form's fields.
    fn response(status: StatusCode, message: &str) -> Response {
        let problem = Self {
            message: message.to_owned(),
            fields: BTreeMap::new(),
        };
        (status, Json(problem)).into_response()
    }

    /// The answer for anything the stub does not hold.
    fn not_found() -> Response {
        Self::response(StatusCode::NOT_FOUND, "not found")
    }
}

/// Which files of the patch to return.
#[derive(Deserialize, IntoParams)]
struct PatchRange {
    /// The index of the first file. 0 when absent.
    offset: Option<usize>,
    /// How many files. All of them when absent.
    limit: Option<usize>,
}

/// Names one file of the change.
#[derive(Deserialize, IntoParams)]
struct FileQuery {
    path: String,
}

/// Everything the stub remembers.
struct Review {
    files: Vec<FilePatch>,
    /// Another version of some files, which a simulated revision swaps in.
    alternates: HashMap<String, FilePatch>,
    revisions: HashMap<String, u32>,
    reviewed: HashSet<String>,
    threads: Vec<Thread>,
    next_id: u32,
    account: Account,
}

impl Review {
    /// Loads the fixture and seeds a thread on every twelfth file with additions.
    fn load(fixture: &str) -> anyhow::Result<Self> {
        let read = |name: &str| {
            let path = format!("{fixture}/{name}");
            std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))
        };
        let files = FilePatch::parse_all(&read("change-1.patch")?);
        let current: HashMap<&str, &str> = files
            .iter()
            .map(|file| (file.path.as_str(), file.text.as_str()))
            .collect();
        let alternates = FilePatch::parse_all(&read("change-1-alt.patch")?)
            .into_iter()
            .filter(|file| {
                current
                    .get(file.path.as_str())
                    .is_some_and(|text| *text != file.text)
            })
            .map(|file| (file.path.clone(), file))
            .collect();

        let mut review = Self {
            files,
            alternates,
            revisions: HashMap::new(),
            reviewed: HashSet::new(),
            threads: Vec::new(),
            next_id: 1,
            account: Account {
                display_name: "Mira Okafor".to_owned(),
                email: "mira@example.com".to_owned(),
                bio: "Reviews the storage layer.".to_owned(),
                notify_on_review: true,
                other_sessions: 3,
            },
        };
        let seeds: Vec<(String, u32)> = review
            .files
            .iter()
            .filter(|file| !file.added_lines.is_empty())
            .step_by(12)
            .map(|file| (file.path.clone(), file.added_lines[0]))
            .collect();
        for (path, line) in seeds {
            let thread = review.start_thread(path, Side::Additions, line, "tomas", SEED_QUESTION);
            let id = thread.id;
            review.reply(id, "ines", SEED_ANSWER);
        }
        Ok(review)
    }

    /// Hands out the next thread or comment id.
    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// A comment written now.
    fn comment(&mut self, author: &str, body: &str) -> Comment {
        Comment {
            id: self.id(),
            author: author.to_owned(),
            body: body.to_owned(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs()),
        }
    }

    fn start_thread(&mut self, path: String, side: Side, line: u32, author: &str, body: &str) -> Thread {
        let thread = Thread {
            id: self.id(),
            path,
            side,
            line,
            comments: vec![self.comment(author, body)],
        };
        self.threads.push(thread.clone());
        thread
    }

    /// Adds a reply, or says the thread does not exist.
    fn reply(&mut self, thread: u32, author: &str, body: &str) -> Option<Comment> {
        let comment = self.comment(author, body);
        let thread = self.threads.iter_mut().find(|candidate| candidate.id == thread)?;
        thread.comments.push(comment.clone());
        Some(comment)
    }

    fn summary(&self, file: &FilePatch) -> FileSummary {
        FileSummary {
            path: file.path.clone(),
            old_path: file.old_path.clone(),
            status: file.status,
            additions: file.additions,
            deletions: file.deletions,
            reviewed: self.reviewed.contains(&file.path),
            revision: self.revisions.get(&file.path).copied().unwrap_or(0),
        }
    }

    /// Swaps a file for its other version, as a push from the author would.
    fn revise(&mut self, path: Option<&str>) -> Option<FileSummary> {
        let index = self.files.iter().position(|file| match path {
            Some(path) => file.path == path,
            None => self.alternates.contains_key(&file.path),
        })?;
        let path = self.files[index].path.clone();
        let other = self.alternates.remove(&path)?;
        let previous = std::mem::replace(&mut self.files[index], other);
        self.alternates.insert(path.clone(), previous);
        *self.revisions.entry(path.clone()).or_default() += 1;
        // A new version needs another look.
        self.reviewed.remove(&path);
        Some(self.summary(&self.files[index]))
    }
}

const SEED_QUESTION: &str = "Does this still hold when the caller retries? I could not find a test for it.";
const SEED_ANSWER: &str = "It does: the second attempt reuses the first one's key. I'll add the test.";

/// The handlers' state.
#[derive(Clone)]
struct Stub {
    review: Arc<Mutex<Review>>,
    events: broadcast::Sender<ChangeEvent>,
}

impl Stub {
    /// The review, or a 404 for any change but the one the stub holds.
    fn review(&self, change: &str) -> Result<MutexGuard<'_, Review>, Response> {
        if change != CHANGE {
            return Err(Problem::not_found());
        }
        self.review
            .lock()
            .map_err(|_| Problem::response(StatusCode::INTERNAL_SERVER_ERROR, "state poisoned"))
    }

    /// Tells every open page. Nobody listening is not an error.
    fn publish(&self, event: ChangeEvent) {
        drop(self.events.send(event));
    }
}

/// A unified diff as a response.
fn diff(text: String) -> Response {
    ([(header::CONTENT_TYPE, "text/x-diff; charset=utf-8")], text).into_response()
}

/// The change, with every file's counts and no diff.
#[utoipa::path(get, path = "/changes/{change}", params(("change" = String, Path)),
    responses((status = 200, body = ChangeDetail), (status = 404, body = Problem)))]
async fn get_change(State(stub): State<Stub>, Path(change): Path<String>) -> Result<Json<ChangeDetail>, Response> {
    let review = stub.review(&change)?;
    let files: Vec<FileSummary> = review.files.iter().map(|file| review.summary(file)).collect();
    Ok(Json(ChangeDetail {
        id: change,
        title: "Bring the engineering standard into CI".to_owned(),
        description: "Everything that landed on main between 1f2f4a231 and 386525cd2, as one change."
            .to_owned(),
        author: "ines".to_owned(),
        base: "1f2f4a231".to_owned(),
        head: "386525cd2".to_owned(),
        additions: files.iter().map(|file| file.additions).sum(),
        deletions: files.iter().map(|file| file.deletions).sum(),
        files,
    }))
}

/// The change's diff as a git patch: all of it, or a range of its files.
#[utoipa::path(get, path = "/changes/{change}/patch", params(("change" = String, Path), PatchRange),
    responses((status = 200, body = String, content_type = "text/x-diff"), (status = 404, body = Problem)))]
async fn get_patch(
    State(stub): State<Stub>,
    Path(change): Path<String>,
    Query(range): Query<PatchRange>,
) -> Result<Response, Response> {
    let review = stub.review(&change)?;
    let files = review.files.iter().skip(range.offset.unwrap_or(0));
    let text = files
        .take(range.limit.unwrap_or(usize::MAX))
        .map(|file| file.text.as_str())
        .collect();
    Ok(diff(text))
}

/// One file's diff as a git patch, at its latest revision.
#[utoipa::path(get, path = "/changes/{change}/file-patch", params(("change" = String, Path), FileQuery),
    responses((status = 200, body = String, content_type = "text/x-diff"), (status = 404, body = Problem)))]
async fn get_file_patch(
    State(stub): State<Stub>,
    Path(change): Path<String>,
    Query(query): Query<FileQuery>,
) -> Result<Response, Response> {
    let review = stub.review(&change)?;
    let file = review.files.iter().find(|file| file.path == query.path);
    file.map(|file| diff(file.text.clone())).ok_or_else(Problem::not_found)
}

/// Every comment thread of the change.
#[utoipa::path(get, path = "/changes/{change}/threads", params(("change" = String, Path)),
    responses((status = 200, body = Vec<Thread>), (status = 404, body = Problem)))]
async fn list_threads(State(stub): State<Stub>, Path(change): Path<String>) -> Result<Json<Vec<Thread>>, Response> {
    Ok(Json(stub.review(&change)?.threads.clone()))
}

/// Starts a thread on a line.
#[utoipa::path(post, path = "/changes/{change}/threads", params(("change" = String, Path)),
    request_body = NewThread,
    responses((status = 201, body = Thread), (status = 404, body = Problem), (status = 422, body = Problem)))]
async fn create_thread(
    State(stub): State<Stub>,
    Path(change): Path<String>,
    Json(new): Json<NewThread>,
) -> Result<(StatusCode, Json<Thread>), Response> {
    let body = new.body.trim();
    if body.is_empty() {
        return Err(Problem::response(StatusCode::UNPROCESSABLE_ENTITY, "a comment needs some text"));
    }
    let thread = stub.review(&change)?.start_thread(new.path, new.side, new.line, ME, body);
    stub.publish(ChangeEvent::ThreadCreated { thread: thread.clone() });
    Ok((StatusCode::CREATED, Json(thread)))
}

/// Replies to a thread.
#[utoipa::path(post, path = "/changes/{change}/threads/{thread}/comments",
    params(("change" = String, Path), ("thread" = u32, Path)), request_body = NewComment,
    responses((status = 201, body = Comment), (status = 404, body = Problem), (status = 422, body = Problem)))]
async fn add_comment(
    State(stub): State<Stub>,
    Path((change, thread)): Path<(String, u32)>,
    Json(new): Json<NewComment>,
) -> Result<(StatusCode, Json<Comment>), Response> {
    let body = new.body.trim();
    if body.is_empty() {
        return Err(Problem::response(StatusCode::UNPROCESSABLE_ENTITY, "a comment needs some text"));
    }
    let comment = stub.review(&change)?.reply(thread, ME, body).ok_or_else(Problem::not_found)?;
    stub.publish(ChangeEvent::CommentAdded { thread_id: thread, comment: comment.clone() });
    Ok((StatusCode::CREATED, Json(comment)))
}

/// Marks a file as reviewed, or takes the mark away.
#[utoipa::path(put, path = "/changes/{change}/reviewed", params(("change" = String, Path)),
    request_body = ReviewedMark,
    responses((status = 200, body = ReviewedMark), (status = 404, body = Problem)))]
async fn set_reviewed(
    State(stub): State<Stub>,
    Path(change): Path<String>,
    Json(mark): Json<ReviewedMark>,
) -> Result<Json<ReviewedMark>, Response> {
    let mut review = stub.review(&change)?;
    if mark.reviewed {
        review.reviewed.insert(mark.path.clone());
    } else {
        review.reviewed.remove(&mark.path);
    }
    Ok(Json(mark))
}

/// What happens to the change from now on, as server-sent events. Each event's
/// data is one `ChangeEvent` as JSON.
#[utoipa::path(get, path = "/changes/{change}/events", params(("change" = String, Path)),
    responses((status = 200, body = ChangeEvent, content_type = "text/event-stream"), (status = 404, body = Problem)))]
async fn stream_events(
    State(stub): State<Stub>,
    Path(change): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Response> {
    drop(stub.review(&change)?);
    // A page that fell behind skips what it missed: the stub keeps no history.
    let events = BroadcastStream::new(stub.events.subscribe())
        .filter_map(|event| event.ok())
        .filter_map(|event| Event::default().json_data(event).ok())
        .map(Ok);
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

/// Makes an update arrive as if someone else made it: a new thread, or a new
/// revision of a file. Only the stub has this.
#[utoipa::path(post, path = "/changes/{change}/simulate", params(("change" = String, Path)),
    request_body = Simulate,
    responses((status = 202), (status = 404, body = Problem)))]
async fn simulate(
    State(stub): State<Stub>,
    Path(change): Path<String>,
    Json(ask): Json<Simulate>,
) -> Result<StatusCode, Response> {
    let mut review = stub.review(&change)?;
    let event = match ask.kind {
        SimulateKind::Thread => {
            let threaded: HashSet<&str> = review.threads.iter().map(|thread| thread.path.as_str()).collect();
            let target = review.files.iter().find(|file| match &ask.path {
                Some(path) => file.path == *path,
                None => !threaded.contains(file.path.as_str()),
            });
            let target = target.filter(|file| !file.added_lines.is_empty());
            let (path, line) = target
                .map(|file| (file.path.clone(), file.added_lines[file.added_lines.len() / 2]))
                .ok_or_else(Problem::not_found)?;
            let body = "Arrived while you were reading: can this move behind the feature flag?";
            ChangeEvent::ThreadCreated {
                thread: review.start_thread(path, Side::Additions, line, "tomas", body),
            }
        }
        SimulateKind::Revision => ChangeEvent::FileRevised {
            file: review.revise(ask.path.as_deref()).ok_or_else(Problem::not_found)?,
        },
    };
    stub.publish(event);
    Ok(StatusCode::ACCEPTED)
}

/// The caller's account settings.
#[utoipa::path(get, path = "/account", responses((status = 200, body = Account)))]
async fn get_account(State(stub): State<Stub>) -> Result<Json<Account>, Response> {
    Ok(Json(stub.review(CHANGE)?.account.clone()))
}

/// Saves the account settings, or says which fields are wrong.
#[utoipa::path(put, path = "/account", request_body = AccountUpdate,
    responses((status = 200, body = Account), (status = 422, body = Problem)))]
async fn update_account(State(stub): State<Stub>, Json(update): Json<AccountUpdate>) -> Result<Json<Account>, Response> {
    let mut fields = BTreeMap::new();
    let name = update.display_name.trim();
    if name.is_empty() || name.chars().count() > 40 {
        fields.insert("display_name".to_owned(), "Use between 1 and 40 characters.".to_owned());
    }
    if !update.email.contains('@') || update.email.ends_with("@example.org") {
        fields.insert("email".to_owned(), "This address cannot receive mail.".to_owned());
    }
    if update.bio.chars().count() > 160 {
        fields.insert("bio".to_owned(), "Keep it under 160 characters.".to_owned());
    }
    if !fields.is_empty() {
        let problem = Problem { message: "Some fields need another look.".to_owned(), fields };
        return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(problem)).into_response());
    }
    let mut review = stub.review(CHANGE)?;
    review.account = Account {
        display_name: name.to_owned(),
        email: update.email,
        bio: update.bio,
        notify_on_review: update.notify_on_review,
        other_sessions: review.account.other_sessions,
    };
    Ok(Json(review.account.clone()))
}

/// Signs every other browser out. The page asks for confirmation first.
#[utoipa::path(delete, path = "/account/sessions", responses((status = 200, body = Account)))]
async fn revoke_other_sessions(State(stub): State<Stub>) -> Result<Json<Account>, Response> {
    let mut review = stub.review(CHANGE)?;
    review.account.other_sessions = 0;
    Ok(Json(review.account.clone()))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (router, openapi) = OpenApiRouter::new()
        .routes(routes!(get_change))
        .routes(routes!(get_patch))
        .routes(routes!(get_file_patch))
        .routes(routes!(list_threads, create_thread))
        .routes(routes!(add_comment))
        .routes(routes!(set_reviewed))
        .routes(routes!(stream_events))
        .routes(routes!(simulate))
        .routes(routes!(get_account, update_account))
        .routes(routes!(revoke_other_sessions))
        .split_for_parts();

    let mut arguments = std::env::args().skip(1);
    let first = arguments.next();
    if first.as_deref() == Some("--openapi") {
        println!("{}", openapi.to_pretty_json()?);
        return Ok(());
    }

    let fixture = first.unwrap_or_else(|| "fixture".to_owned());
    let stub = Stub {
        review: Arc::new(Mutex::new(Review::load(&fixture)?)),
        events: broadcast::channel(64).0,
    };
    let address = std::env::var("STUB_API_ADDRESS").unwrap_or_else(|_| "127.0.0.1:4100".to_owned());
    let listener = tokio::net::TcpListener::bind(&address).await.with_context(|| format!("binding {address}"))?;
    eprintln!("stub API on http://{address}");
    axum::serve(listener, router.with_state(stub).layer(CorsLayer::permissive())).await?;
    Ok(())
}

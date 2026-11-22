//! Focused persistence and security contract tests.
use crate::Database;

use crate::repositories::{
    SessionRepository, UserRepository,
    sessions::{InsertSession, LAST_SEEN_INTERVAL, SessionScope},
    users::InsertUser,
};
use chrono::Utc;
use sourceorigin_common::{Handle, UserId};
use std::{net::IpAddr, time::Duration};

/// Inserts a passkey row, which references the account through a foreign key.
async fn insert_reference(connection: &mut sqlx::PgConnection, user: UserId) {
    sqlx::query!(
        "INSERT INTO passkey_credentials(id,credential_id,user_id,credential,friendly_name)
        VALUES($1,$2,$3,'{}','Lock test')",
        uuid::Uuid::now_v7(),
        uuid::Uuid::now_v7().as_bytes().to_vec(),
        user.as_uuid()
    )
    .execute(connection)
    .await
    .expect("reference");
}

/// Account locks let other transactions insert rows referencing the account.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn account_locks_allow_foreign_key_checks() {
    let pool = Database::test_pool().await;
    let user = UserRepository::from_pool(&pool).insert_named("lock").await;
    let mut account = pool.begin().await.expect("account transaction");
    UserRepository::from_transaction(&mut account)
        .find_active_by_id_for_update(user)
        .await
        .expect("account lock")
        .expect("active account");
    let mut reference = pool.begin().await.expect("reference transaction");
    tokio::time::timeout(
        Duration::from_secs(2),
        insert_reference(&mut reference, user),
    )
    .await
    .expect("account lock must allow foreign-key checks");
    reference.rollback().await.expect("rollback reference");
    account.rollback().await.expect("rollback account");
}

/// Session creation and email claims lock the account too. They must not wait on a
/// concurrent foreign-key check, or they deadlock with requests that reference it.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn account_locks_do_not_wait_on_foreign_key_checks() {
    let pool = Database::test_pool().await;
    let user = UserRepository::from_pool(&pool).insert_named("lock").await;
    let mut reference = pool.begin().await.expect("reference transaction");
    insert_reference(&mut reference, user).await;
    let mut account = pool.begin().await.expect("account transaction");
    tokio::time::timeout(
        Duration::from_secs(2),
        SessionRepository::from_transaction(&mut account)
            .insert(&InsertSession::lasting(user, chrono::Duration::days(1))),
    )
    .await
    .expect("session creation must not wait on foreign-key checks")
    .expect("session");
    tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query!(
            "INSERT INTO user_emails (id, user_id, normalized_email, verified_at)
            VALUES ($1, $2, $3, now())",
            uuid::Uuid::now_v7(),
            user.as_uuid(),
            format!("{user}@example.com")
        )
        .execute(&mut *account),
    )
    .await
    .expect("email claims must not wait on foreign-key checks")
    .expect("email");
    account.rollback().await.expect("rollback account");
    reference.rollback().await.expect("rollback reference");
}

/// Code exchange revalidates its session under lock, so it sees a concurrent logout.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn locked_session_lookup_waits_for_concurrent_logout() {
    let pool = Database::test_pool().await;
    let user = UserRepository::from_pool(&pool).insert_named("lock").await;
    let session = SessionRepository::from_pool(&pool)
        .insert(&InsertSession::lasting(user, chrono::Duration::days(1)))
        .await
        .expect("session")
        .id;
    let mut logout = pool.begin().await.expect("logout transaction");
    assert_eq!(
        SessionRepository::from_transaction(&mut logout)
            .revoke(user, SessionScope::One(session))
            .await
            .expect("revoke"),
        [session]
    );
    let mut exchange = pool.begin().await.expect("exchange transaction");
    let backend = Database::backend(&mut exchange).await;
    let exchange = tokio::spawn(async move {
        SessionRepository::from_transaction(&mut exchange)
            .find_active_by_id_for_update(session)
            .await
            .expect("lookup")
    });
    // The lookup waits for the logout.
    Database::wait_for_lock(&pool, backend).await;
    logout.commit().await.expect("commit logout");
    assert!(exchange.await.expect("exchange task").is_none());
}

/// Activity is recorded at most once per interval: a fresh session keeps its first
/// IP, and a stale one takes the new one.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn touch_records_activity_at_most_once_per_interval() {
    let pool = Database::test_pool().await;
    let user = UserRepository::from_pool(&pool).insert_named("lock").await;
    let mut sessions = SessionRepository::from_pool(&pool);
    let session = sessions
        .insert(&InsertSession::lasting(user, chrono::Duration::days(1)))
        .await
        .expect("session");
    let elsewhere = IpAddr::from([198, 51, 100, 9]);
    let pool = &pool;
    let last_seen = |id| async move {
        let session = SessionRepository::from_pool(pool)
            .find_by_id(id)
            .await
            .expect("lookup")
            .expect("session");
        (session.last_seen_at, session.last_seen_ip)
    };

    sessions.touch(session.id, elsewhere).await.expect("touch");
    assert_eq!(
        last_seen(session.id).await,
        (session.last_seen_at, session.last_seen_ip),
        "a fresh session is not written"
    );

    sqlx::query!(
        "UPDATE sessions SET last_seen_at = $2 WHERE id = $1",
        session.id.as_uuid(),
        Utc::now() - LAST_SEEN_INTERVAL - chrono::Duration::seconds(1)
    )
    .execute(pool)
    .await
    .expect("age session");
    sessions.touch(session.id, elsewhere).await.expect("touch");
    let (at, ip) = last_seen(session.id).await;
    assert!(at > session.last_seen_at);
    assert_eq!(ip, Some(elsewhere));
}

/// A search lists the closest accounts first: the one whose handle is the
/// query, then handles starting with it, then display names with a word
/// starting with it, then the rest.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn search_lists_the_closest_accounts_first() {
    let pool = Database::test_pool().await;
    // Unique to this test, so accounts other tests insert never match.
    let query = format!("rank{}", &UserId::new().to_string().replace('-', "")[..10]);
    let account = |handle: String, display_name: String| InsertUser {
        id: UserId::new(),
        handle: Handle::parse(&handle).expect("a handle"),
        display_name,
    };
    let expected = [
        account(query.clone(), "Exact".to_owned()),
        account(format!("{query}-b"), "Prefix".to_owned()),
        account(format!("named-{query}x"), format!("Someone {query}")),
        account(format!("within{query}"), "Within".to_owned()),
    ];
    // Inserted in reverse, so the order is the search's, not the insertion's.
    for input in expected.iter().rev() {
        UserRepository::from_pool(&pool)
            .insert(input)
            .await
            .expect("user");
    }
    let found = UserRepository::from_pool(&pool)
        .search(&query.to_uppercase(), 0, 10)
        .await
        .expect("search");
    let found: Vec<_> = found.iter().map(|user| user.id).collect();
    let expected: Vec<_> = expected.iter().map(|input| input.id).collect();
    assert_eq!(found, expected);
    // An ID matches whole, in either case, as one pasted from elsewhere.
    let pasted = expected[1].to_string().to_uppercase();
    let found = UserRepository::from_pool(&pool)
        .search(&pasted, 0, 10)
        .await
        .expect("search by ID");
    assert_eq!(found.first().map(|user| user.id), Some(expected[1]));
}

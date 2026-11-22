//! Advisory lock exclusion and release.

use super::LockKey;
use crate::Database;
use std::time::Duration;

/// One session at a time holds a key's lock. Releasing frees it at once. Dropping
/// the holder, as cancelling its job does, frees it once `PostgreSQL` ends the
/// session.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn locks_exclude_other_sessions_until_their_connection_ends() {
    let database = Database::test().await;
    let lock = || database.try_exclusive(LockKey::LockTest);
    let holder = lock().await.expect("lock").expect("a free lock");
    assert!(lock().await.expect("lock").is_none(), "the lock is held");
    holder.release().await.expect("release");
    let holder = lock().await.expect("lock").expect("a released lock");
    let job = tokio::spawn(async move {
        let _holder = holder;
        std::future::pending::<()>().await;
    });
    job.abort();
    assert!(job.await.expect_err("cancelled job").is_cancelled());
    // PostgreSQL grants the waiting lock once the dropped holder's session ends.
    let freed = tokio::time::timeout(
        Duration::from_secs(5),
        database.exclusive(LockKey::LockTest),
    )
    .await
    .expect("dropping the holder frees the lock")
    .expect("lock");
    freed.release().await.expect("release");
}

/// Transactions on a locked connection read committed data and may write, whatever default
/// the session has, as the audit chain's seals must.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn locked_transactions_read_committed_data() {
    let database = Database::test().await;
    let mut holder = database
        .try_exclusive(LockKey::TransactionTest)
        .await
        .expect("lock")
        .expect("a free lock");
    sqlx::raw_sql(
        "SET default_transaction_isolation = 'repeatable read';
        SET default_transaction_read_only = on",
    )
    .execute(&mut holder.connection)
    .await
    .expect("session defaults");
    let mut tx = holder.begin().await.expect("transaction");
    let settings = sqlx::query!(
        r#"SELECT current_setting('transaction_isolation') AS "isolation!",
            current_setting('transaction_read_only') AS "read_only!""#
    )
    .fetch_one(&mut *tx.inner)
    .await
    .expect("settings");
    assert_eq!(
        (settings.isolation.as_str(), settings.read_only.as_str()),
        ("read committed", "off")
    );
    tx.rollback().await.expect("rollback");
    holder.release().await.expect("release");
}

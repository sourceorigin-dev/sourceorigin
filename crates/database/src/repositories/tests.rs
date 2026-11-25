//! Stored values that their Rust types reject surface as invalid data, and
//! transaction-scoped locks refuse to run outside a transaction.
use crate::{
    Database, DatabaseError, LockKey,
    repositories::{SessionRepository, UserRepository},
};
use sourceorigin_common::{Handle, SessionId, UserId};

/// A non-`UUIDv7` ID and JSON outside its record's type both fail decoding as
/// [`DatabaseError::InvalidStoredData`], not as a generic operation failure.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn undecodable_columns_are_invalid_stored_data() {
    let pool = Database::test_pool().await;
    let v4 = uuid::Builder::from_random_bytes(*uuid::Uuid::now_v7().as_bytes()).into_uuid();
    let handle = Handle::parse(&format!("v4-{}", &v4.simple().to_string()[20..])).expect("handle");
    sqlx::query!(
        "WITH account AS (INSERT INTO users(id,display_name) VALUES($1,'Decode test'))
        INSERT INTO handles(handle,user_id) VALUES($2,$1)",
        v4,
        handle.as_str()
    )
    .execute(&pool)
    .await
    .expect("user with a v4 id");
    assert!(matches!(
        UserRepository::from_pool(&pool)
            .find_active_by_handle(&handle)
            .await,
        Err(DatabaseError::InvalidStoredData { .. })
    ));

    let user = UserId::new();
    let session = SessionId::new();
    sqlx::query!(
        "INSERT INTO users(id,display_name) VALUES($1,'Decode test')",
        user.as_uuid()
    )
    .execute(&pool)
    .await
    .expect("user");
    sqlx::query!("INSERT INTO sessions(id,user_id,authentication_methods,authenticated_at,expires_at,last_seen_at)
        VALUES($1,$2,'[\"unknown\"]',now(),now()+interval '1 day',now())", session.as_uuid(), user.as_uuid())
    .execute(&pool)
    .await
    .expect("session with an unknown method");
    assert!(matches!(
        SessionRepository::from_pool(&pool)
            .find_by_id(session)
            .await,
        Err(DatabaseError::InvalidStoredData { .. })
    ));
}

/// A transaction-scoped advisory lock taken on the pool or an exclusive connection
/// would end with its statement, so both refuse it before running any query; a
/// transaction takes it.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn transaction_scoped_locks_need_a_transaction() {
    let database = Database::test().await;
    assert!(matches!(
        database.oidc().lock_related_origins().await,
        Err(DatabaseError::OutsideTransaction)
    ));

    let mut tx = database.begin().await.expect("transaction");
    tx.oidc().lock_related_origins().await.expect("lock");
    tx.rollback().await.expect("rollback");

    let mut exclusive = database
        .try_exclusive(LockKey::GuardTest)
        .await
        .expect("lock")
        .expect("a free lock");
    assert!(matches!(
        exclusive.oidc().lock_related_origins().await,
        Err(DatabaseError::OutsideTransaction)
    ));
    exclusive.release().await.expect("release");
}

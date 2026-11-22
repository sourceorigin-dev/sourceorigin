//! Focused persistence and security contract tests.
use super::InsertUnverifiedEmail;
use crate::{Database, UserRepository};
use chrono::{Duration, Utc};
use sourceorigin_common::{EmailId, UserId};
/// Verified email ownership follows verification changes.
#[expect(
    clippy::too_many_lines,
    reason = "Keep the ownership lifecycle scenario of one address together."
)]
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn verified_email_updates_preserve_and_release_ownership() {
    let pool = Database::test_pool().await;
    let mut tx = pool.begin().await.expect("transaction");
    let mut accounts = UserRepository::from_transaction(&mut tx);
    let users = [
        accounts.insert_named("email").await,
        accounts.insert_named("email").await,
    ];
    let email = uuid::Uuid::now_v7();
    let address = format!("{email}@example.test");
    sqlx::query!(
        "INSERT INTO user_emails (id, user_id, normalized_email, verified_at, is_primary)
        VALUES ($1, $2, $3, now(), true)",
        email,
        users[0].as_uuid(),
        &address
    )
    .execute(&mut *tx)
    .await
    .expect("email");
    #[expect(
        clippy::disallowed_methods,
        reason = "The `SET` clause is the loop's variable"
    )]
    for update in [
        "verified_at=verified_at",
        "verified_at=now()+interval '1 second'",
    ] {
        sqlx::query(&format!("UPDATE user_emails SET {update} WHERE id=$1"))
            .bind(email)
            .execute(&mut *tx)
            .await
            .expect("update verification");
        let owner = sqlx::query_scalar!(
            "SELECT user_id FROM verified_email_ownership WHERE normalized_email=$1",
            &address,
        )
        .fetch_one(&mut *tx)
        .await
        .expect("ownership retained");
        assert_eq!(owner, users[0].as_uuid());
    }
    sqlx::query!("SAVEPOINT duplicate_email")
        .execute(&mut *tx)
        .await
        .expect("savepoint");
    sqlx::query!(
        "INSERT INTO user_emails (id, user_id, normalized_email, verified_at, is_primary)
            VALUES ($1, $2, $3, now(), true)",
        uuid::Uuid::now_v7(),
        users[1].as_uuid(),
        &address
    )
    .execute(&mut *tx)
    .await
    .expect_err("should be rejected");
    sqlx::query!("ROLLBACK TO duplicate_email")
        .execute(&mut *tx)
        .await
        .expect("rollback duplicate");
    sqlx::query!(
        "UPDATE user_emails SET normalized_email=normalized_email||'.new',user_id=$2 WHERE id=$1",
        email,
        users[1].as_uuid()
    )
    .execute(&mut *tx)
    .await
    .expect("move address and owner");
    let old_exists = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM verified_email_ownership WHERE normalized_email=$1) AS "exists!""#,
        &address,
    )
    .fetch_one(&mut *tx)
    .await
    .expect("old ownership");
    assert!(!old_exists);
    sqlx::query!(
        "UPDATE user_emails SET verified_at=NULL,is_primary=false WHERE id=$1",
        email
    )
    .execute(&mut *tx)
    .await
    .expect("unverify");
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM verified_email_ownership WHERE user_id=ANY($1)"#,
        &users.map(UserId::as_uuid)[..],
    )
    .fetch_one(&mut *tx)
    .await
    .expect("ownership count");
    assert_eq!(count, 0);
    sqlx::query!(
        "UPDATE user_emails SET verified_at=now(),is_primary=true WHERE id=$1",
        email
    )
    .execute(&mut *tx)
    .await
    .expect("reverify");
    sqlx::query!("DELETE FROM user_emails WHERE id=$1", email)
        .execute(&mut *tx)
        .await
        .expect("delete");
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM verified_email_ownership WHERE user_id=ANY($1)"#,
        &users.map(UserId::as_uuid)[..],
    )
    .fetch_one(&mut *tx)
    .await
    .expect("ownership count");
    assert_eq!(count, 0);
    sqlx::query!("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *tx)
        .await
        .expect("primary invariants");
    tx.rollback().await.expect("rollback");
}

/// Cleanup deletes an unverified address once its verification link expired, and
/// keeps addresses whose link still works and verified ones.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn cleanup_deletes_unverified_addresses_with_expired_links() {
    let database = Database::test().await;
    let user_id = database.users().insert_named("cleanup").await;
    let [expired, pending] = [EmailId::new(), EmailId::new()];
    for (id, expires_at) in [
        (expired, Utc::now() - Duration::seconds(1)),
        (pending, Utc::now() + Duration::hours(1)),
    ] {
        database
            .emails()
            .insert_unverified(&InsertUnverifiedEmail {
                id,
                user_id,
                normalized_email: &format!("{id}@example.test"),
                secret_digest: id.as_uuid().as_u128().to_be_bytes().repeat(2).as_slice(),
                expires_at,
            })
            .await
            .expect("unverified address");
    }
    database
        .maintenance()
        .purge_expired_auth_state(1000)
        .await
        .expect("purge");
    let remaining: Vec<_> = database
        .emails()
        .list(user_id)
        .await
        .expect("addresses")
        .into_iter()
        .map(|email| email.id)
        .collect();
    assert_eq!(remaining, [pending]);
}

//! How a change waits on locks and rechecks its caller after waiting.
//!
//! Each scenario holds a lock while a mutation waits for it, and every mutation
//! first takes the administrators lock, so each runs in a database of its own: no
//! other test's change queues behind the lock it holds, or holds one it waits for.

use super::*;
use crate::security::CONFIRMATION_MAX_AGE;

/// A mutation checks its caller's confirmation after its last write: one admitted
/// just inside the window, that then waited for a logout holding the target's
/// session, is refused with the step-up and changes nothing.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn mutations_recheck_the_confirmation_after_waiting() {
    let harness = Harness::isolated().await;
    let admin = harness.admin().await;
    let user = harness.account().await;
    let mut logout = harness.database.begin().await.expect("transaction");
    user.sign_out(&mut logout).await;
    let service = harness.service();
    let reason = reason("Spam");
    // The confirmation ends two seconds from now, and the logout ends just after
    // it, so the mutation waits about two seconds, well within the lock wait limit.
    let window = Duration::seconds(CONFIRMATION_MAX_AGE.into());
    let auth_time = Utc::now() - window + Duration::seconds(2);
    let operator = operator(admin, auth_time);
    let (result, ()) = tokio::join!(
        service.disable_user(&operator, user.user_id, reason),
        async {
            until_blocked(&mut logout, 1).await;
            assert!(
                operator.confirmation_left() > Duration::zero(),
                "the mutation waits while its caller is still confirmed"
            );
            let expired = auth_time + window + Duration::milliseconds(10) - Utc::now();
            tokio::time::sleep(expired.to_std().unwrap_or_default()).await;
            logout.commit().await.expect("logout");
        }
    );
    assert!(
        matches!(result, Err(AdminError::StepUpRequired(_))),
        "{result:?}"
    );
    let stored = harness
        .database
        .users()
        .find_by_id(user.user_id)
        .await
        .expect("lookup")
        .expect("user");
    assert_eq!(stored.status, AccountStatus::Active);
}

/// A mutation waiting for its target holds no lock of its caller's, so a sign-in
/// that locks the target's account and then revokes the caller's session, as
/// switching accounts does, finishes first.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn waiting_mutations_hold_no_caller_locks() {
    let harness = Harness::isolated().await;
    let admin = harness.admin().await;
    let user = harness.account().await;
    let mut switch = harness.database.begin().await.expect("transaction");
    switch
        .users()
        .find_active_by_id_for_update(user.user_id)
        .await
        .expect("target lock");
    let service = harness.service();
    let reason = reason("Spam");
    let operator = operator(admin, Utc::now());
    let (result, ()) = tokio::join!(
        service.disable_user(&operator, user.user_id, reason),
        async {
            until_blocked(&mut switch, 1).await;
            // The switch revokes the caller's session without a deadlock.
            admin.sign_out(&mut switch).await;
            switch.commit().await.expect("commit");
        }
    );
    assert!(
        matches!(result, Err(AdminError::SessionEnded)),
        "{result:?}"
    );
}

/// A mutation never waits for its caller's session, which the caller's own
/// sign-in or social linking may hold while it waits for the mutation's locks.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn mutations_never_wait_for_the_callers_session() {
    let harness = Harness::isolated().await;
    let admin = harness.admin().await;
    let user = harness.account().await;
    let mut linking = harness.database.begin().await.expect("transaction");
    linking
        .sessions()
        .find_active_by_id_for_update(admin.session_id)
        .await
        .expect("caller session lock");
    let reason = reason("Spam");
    let disabled = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        harness
            .service()
            .disable_user(&operator(admin, Utc::now()), user.user_id, reason),
    )
    .await
    .expect("the mutation does not wait for the caller's session");
    assert!(disabled.is_ok(), "{disabled:?}");
    linking.rollback().await.expect("rollback");
}

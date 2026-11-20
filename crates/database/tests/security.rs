//! Persistence guarantees behind invitations and session passkey confirmations.
use chrono::{Duration, Utc};
use sourceorigin_common::{
    AuthenticationMethod, PasskeyId, TokenId,
    events::{Actor, EventContext, RequestOrigin, RevocationReason},
};
use sourceorigin_repository::{
    Database, DatabaseError, InsertInvitation, InsertSession, InvitationKey, SessionScope,
};
/// Invitations are single-use, and passkey confirmations belong to one session and end with it.
#[expect(
    clippy::too_many_lines,
    reason = "Keep the invitation rollback and session lifecycle scenario together."
)]
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn invitation_redemption_and_confirmation_lifecycle() {
    let db = Database::test().await;
    let digest = TokenId::new().as_uuid().as_bytes().repeat(2);
    db.security()
        .create_invitation(&InsertInvitation {
            id: TokenId::new(),
            secret_digest: &digest,
            expires_at: Utc::now() + Duration::minutes(1),
            email: None,
        })
        .await
        .expect("invite");
    let mut tx = db.begin().await.expect("transaction");
    assert!(
        tx.security()
            .redeem_invitation(&digest)
            .await
            .expect("redeem")
            .is_some()
    );
    tx.rollback().await.expect("rollback");
    assert!(
        db.security()
            .open_invitation(InvitationKey::Digest(&digest))
            .await
            .expect("rollback preserves invitation")
            .is_some()
    );
    let redeem = || async {
        db.security()
            .redeem_invitation(&digest)
            .await
            .expect("redemption")
            .is_some()
    };
    let (a, b) = tokio::join!(redeem(), redeem());
    assert_ne!(a, b, "exactly one concurrent redemption succeeds");
    let user = db.users().insert_named("test").await;
    let signed_in = || async {
        db.sessions()
            .insert(&InsertSession {
                authentication_methods: vec![AuthenticationMethod::Federation],
                ..InsertSession::lasting(user, Duration::minutes(1))
            })
            .await
            .expect("session")
            .id
    };
    let (session, other) = (signed_in().await, signed_in().await);
    let database = &db;
    let confirmed_at = |id| async move {
        database
            .sessions()
            .find_active_by_id(id)
            .await
            .expect("lookup")
            .and_then(|session| session.passkey_confirmed_at)
    };
    assert!(
        confirmed_at(session).await.is_none(),
        "social sign-in has no confirmation"
    );
    for id in [session, other] {
        db.sessions()
            .confirm_passkey(id, PasskeyId::new(), Utc::now())
            .await
            .expect("confirm");
    }
    let context = EventContext {
        actor: Actor::User {
            user_id: user,
            session_id: Some(session),
        },
        origin: RequestOrigin::default(),
        reason: None,
    };
    let revoke = |scope, reason| {
        let context = &context;
        async move {
            let mut tx = database.begin().await.expect("transaction");
            tx.revoke_sessions(context, user, scope, &reason)
                .await
                .expect("revoke");
            tx.commit().await.expect("commit revocation");
        }
    };
    revoke(
        SessionScope::AllExcept(session),
        RevocationReason::CredentialRemoved,
    )
    .await;
    assert!(
        db.sessions()
            .find_by_id(other)
            .await
            .expect("other")
            .is_some_and(|other| other.revoked_at.is_some() && other.passkey_confirmed_at.is_none()),
        "revoking other sessions clears their confirmation"
    );
    assert!(confirmed_at(session).await.is_some(), "current preserved");
    revoke(SessionScope::One(session), RevocationReason::Logout).await;
    assert!(
        matches!(
            db.sessions()
                .confirm_passkey(session, PasskeyId::new(), Utc::now())
                .await,
            Err(DatabaseError::NotFound)
        ),
        "a revoked session cannot be confirmed"
    );
}

//! Recording and personal data.
use super::*;
use crate::{Database, DatabaseTransaction, UserRepository};
use sourceorigin_common::{
    Provider,
    clients::RelatedOrigin,
    events::{RecordedOrigin, RevocationReason},
};
use sqlx::PgPool;

/// A context with request personal data, acting as `user`.
fn context(user: UserId) -> EventContext {
    EventContext {
        actor: Actor::User {
            user_id: user,
            session_id: None,
        },
        origin: RequestOrigin {
            client_ip: Some(IpAddr::from([192, 0, 2, 7])),
            user_agent_family: Some(UserAgentFamily {
                browser: Browser::Firefox,
                platform: Platform::Linux,
            }),
        },
        reason: Some("Reported for spam".into()),
    }
}

/// Records `event` about `user` in its own committed transaction.
async fn record(pool: &PgPool, user: UserId, event: DomainEvent) {
    let mut tx = DatabaseTransaction::new(pool.begin().await.expect("transaction"));
    tx.record(&context(user), Some(user), event)
        .await
        .expect("record");
    tx.commit().await.expect("commit");
}

/// Stored personal data fills only the payload's `"$erased"` slots: one naming
/// any other value leaves it as recorded.
#[test]
fn personal_data_restores_only_erased_values() {
    let mut payload = serde_json::json!({"provider": "github", "subject": ERASED});
    let details = serde_json::json!({"/provider": "forged", "/subject": "subject-1"});
    serde_json::from_value::<PersonalDetails>(details)
        .expect("details")
        .restore(&mut payload);
    assert_eq!(
        payload,
        serde_json::json!({"provider": "github", "subject": {PERSONAL_KEY: "subject-1"}})
    );
}

/// Events commit and roll back with their transaction. Personal values, the
/// request origin and the reason stay out of the event row, read back while kept,
/// and read as erased once their row is deleted.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn personal_data_is_stored_apart_and_erases() {
    let pool = Database::test_pool().await;
    let user = UserRepository::from_pool(&pool).insert_named("event").await;
    let subject = format!("subject-{user}");
    let linked = || DomainEvent::IdentityLinked {
        provider: Provider::GitHub,
        subject: Personal::Value(subject.clone()),
    };
    let history = || async {
        EventRepository::from_pool(&pool)
            .history_for_user(user, 10)
            .await
            .expect("history")
    };

    let mut tx = DatabaseTransaction::new(pool.begin().await.expect("transaction"));
    tx.record(&context(user), Some(user), linked())
        .await
        .expect("record");
    tx.rollback().await.expect("rollback");
    assert!(
        history().await.is_empty(),
        "a rolled-back change leaves no event"
    );

    record(&pool, user, linked()).await;
    let [stored] = <[StoredEvent; 1]>::try_from(history().await).expect("one event");
    assert_eq!(stored.event, RecordedEvent::Known(linked()));
    assert_eq!(stored.actor.known(), Some(&context(user).actor));
    assert_eq!(stored.origin, Personal::Value(context(user).origin));
    assert_eq!(stored.reason, context(user).reason);
    let rows = sqlx::query!(
        r#"SELECT to_jsonb(e)::text AS "event!", to_jsonb(p)::text AS "personal!"
        FROM domain_events e JOIN event_personal_data p ON p.event_id = e.id
        WHERE e.id = $1"#,
        stored.id.as_uuid(),
    )
    .fetch_one(&pool)
    .await
    .expect("rows");
    let (event_row, personal_row) = (rows.event, rows.personal);
    for personal in [
        subject.as_str(),
        "192.0.2.7",
        "firefox",
        "Reported for spam",
    ] {
        assert!(!event_row.contains(personal), "{personal} in {event_row}");
        assert!(
            personal_row.contains(personal),
            "{personal} in {personal_row}"
        );
    }

    sqlx::query!(
        "DELETE FROM event_personal_data WHERE event_id = $1",
        stored.id.as_uuid()
    )
    .execute(&pool)
    .await
    .expect("erase");
    let [erased] = <[StoredEvent; 1]>::try_from(history().await).expect("one event");
    assert_eq!(
        erased.event,
        RecordedEvent::Known(DomainEvent::IdentityLinked {
            provider: Provider::GitHub,
            subject: Personal::Erased,
        })
    );
    assert_eq!(erased.origin, Personal::Erased);
    assert_eq!(erased.reason, None);
}

/// An event whose payload an older release could not decode is written in its
/// kind's next version, and this release reads it back. Other events of the kind
/// keep the first version.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn versions_are_per_event() {
    let pool = Database::test_pool().await;
    let user = UserRepository::from_pool(&pool).insert_named("event").await;
    let revoked = |reason| DomainEvent::SessionRevoked {
        session_id: SessionId::new(),
        reason,
    };
    let events = [
        revoked(RevocationReason::Logout),
        revoked(RevocationReason::UserRevoked),
        revoked(RevocationReason::SignOutLink),
    ];
    for event in events.clone() {
        record(&pool, user, event).await;
    }
    let stored = EventRepository::from_pool(&pool)
        .history_for_user(user, 10)
        .await
        .expect("history");
    assert_eq!(
        stored
            .iter()
            .rev()
            .map(|stored| &stored.event)
            .collect::<Vec<_>>(),
        events
            .iter()
            .cloned()
            .map(RecordedEvent::Known)
            .collect::<Vec<_>>()
            .iter()
            .collect::<Vec<_>>()
    );
    let versions = sqlx::query_scalar!(
        "SELECT version FROM domain_events WHERE subject_user_id = $1 ORDER BY sequence",
        user.as_uuid(),
    )
    .fetch_all(&pool)
    .await
    .expect("versions");
    assert_eq!(versions, [1, 2, 3]);
}

/// A related origin is personal data, in an event's second payload version.
/// Events that kept it in the payload, before, still read as their first version.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn related_origins_are_personal_but_older_events_still_read() {
    let pool = Database::test_pool().await;
    let user = UserRepository::from_pool(&pool).insert_named("event").await;
    let client_id: ClientId = format!("origins_{user}").parse().expect("client ID");
    let origin: RelatedOrigin = format!("https://{}.example", user.as_uuid().simple())
        .parse()
        .expect("origin");
    let approved = |origin| DomainEvent::RelatedOriginApproved {
        client_id: client_id.clone(),
        origin,
    };
    let mut events = EventRepository::from_pool(&pool);
    events
        .insert_written_elsewhere(
            user,
            "related_origin_approved",
            DomainEvent::FIRST_VERSION,
            &serde_json::json!({"client_id": client_id, "origin": origin}),
            &serde_json::json!({"type": "anonymous"}),
            None,
        )
        .await
        .expect("sealed event");
    record(&pool, user, approved(origin.clone().into())).await;

    let read = || async {
        EventRepository::from_pool(&pool)
            .history_for_user(user, 10)
            .await
            .expect("history")
            .into_iter()
            .map(|stored| stored.event)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        read().await,
        [
            RecordedEvent::Known(approved(origin.clone().into())),
            RecordedEvent::Known(approved(RecordedOrigin::Sealed(origin.clone()))),
        ]
    );
    let payloads = sqlx::query!(
        "SELECT version, payload FROM domain_events WHERE subject_user_id = $1 ORDER BY sequence",
        user.as_uuid(),
    )
    .fetch_all(&pool)
    .await
    .expect("payloads");
    assert_eq!(payloads[1].version, 2);
    assert_eq!(payloads[1].payload["origin"], ERASED);

    sqlx::query!(
        "DELETE FROM event_personal_data p USING domain_events e
        WHERE p.event_id = e.id AND e.subject_user_id = $1",
        user.as_uuid()
    )
    .execute(&pool)
    .await
    .expect("erase");
    assert_eq!(
        read().await[0],
        RecordedEvent::Known(approved(RecordedOrigin::Personal(Personal::Erased)))
    );
}

/// Events a newer release wrote, of a kind this one does not know or a known kind
/// in a newer payload version, read as unknown with their data. A known kind
/// whose payload does not decode still fails.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn events_a_newer_release_wrote_read_as_unknown() {
    let pool = Database::test_pool().await;
    let user = UserRepository::from_pool(&pool).insert_named("event").await;
    let mut events = EventRepository::from_pool(&pool);
    let data = serde_json::json!({"client_id": "deleted"});
    let anonymous = serde_json::json!({"type": "anonymous"});
    for (kind, version) in [
        ("from_a_newer_release", DomainEvent::FIRST_VERSION),
        ("passkey_added", DomainEvent::FIRST_VERSION + 1),
    ] {
        events
            .insert_written_elsewhere(user, kind, version, &data, &anonymous, None)
            .await
            .expect("event");
    }
    let unknown = |kind: &str, version| RecordedEvent::Unknown {
        kind: kind.into(),
        version,
        data: data.clone(),
    };
    assert_eq!(
        events
            .history_for_user(user, 10)
            .await
            .expect("history")
            .into_iter()
            .map(|stored| stored.event)
            .collect::<Vec<_>>(),
        [
            unknown("passkey_added", DomainEvent::FIRST_VERSION + 1),
            unknown("from_a_newer_release", DomainEvent::FIRST_VERSION),
        ]
    );

    let mut tx = DatabaseTransaction::new(pool.begin().await.expect("transaction"));
    tx.events()
        .insert_written_elsewhere(
            user,
            "passkey_added",
            DomainEvent::FIRST_VERSION,
            &data,
            &anonymous,
            None,
        )
        .await
        .expect("malformed event");
    assert!(matches!(
        tx.events().history_for_user(user, 10).await,
        Err(DatabaseError::InvalidStoredData { .. })
    ));
    tx.rollback().await.expect("rollback");
}

/// Actors a newer release recorded, of a type this one does not know or a known
/// type naming a job it does not know, read as unknown with their JSON. A known
/// type that does not decode still fails.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn actors_a_newer_release_wrote_read_as_unknown() {
    let pool = Database::test_pool().await;
    let user = UserRepository::from_pool(&pool).insert_named("event").await;
    let mut events = EventRepository::from_pool(&pool);
    let actors = [
        serde_json::json!({"type": "from_a_newer_release"}),
        serde_json::json!({"type": "system", "job": "from_a_newer_release"}),
    ];
    for actor in &actors {
        events
            .insert_written_elsewhere(
                user,
                "account_exported",
                DomainEvent::FIRST_VERSION,
                &serde_json::json!({}),
                actor,
                None,
            )
            .await
            .expect("event");
    }
    let read: Vec<_> = events
        .history_for_user(user, 10)
        .await
        .expect("history")
        .into_iter()
        .map(|stored| stored.actor)
        .collect();
    let [older, newer] = actors.map(RecordedActor::Unknown);
    assert_eq!(read, [newer, older]);

    let mut tx = DatabaseTransaction::new(pool.begin().await.expect("transaction"));
    tx.events()
        .insert_written_elsewhere(
            user,
            "account_exported",
            DomainEvent::FIRST_VERSION,
            &serde_json::json!({}),
            &serde_json::json!({"type": "user"}),
            None,
        )
        .await
        .expect("malformed actor");
    assert!(matches!(
        tx.events().history_for_user(user, 10).await,
        Err(DatabaseError::InvalidStoredData { .. })
    ));
    tx.rollback().await.expect("rollback");
}

/// Alert update authors a newer release recorded, of a type this one does not
/// know or a known type naming a job it does not know, read as no author. A
/// known type that does not decode still fails.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn alert_authors_a_newer_release_wrote_read_as_unknown() {
    use crate::DatabaseConfig;
    use sourceorigin_common::{
        admin::{ChainBreak, ChainProblem},
        alerts::{Alert, AlertStatus},
    };
    use std::time::Duration;
    let url = Database::test_url();
    let database = Database::isolated(
        DatabaseConfig::new(&url, 1, Duration::from_secs(5)).expect("configuration"),
    )
    .await
    .expect("isolated database");
    let alert = Alert::AuditChainBroken(ChainBreak {
        position: 1,
        problem: ChainProblem::EventChanged,
    });
    let id = database.alerts().raise(&alert).await.expect("alert");
    let post = async |author| {
        database
            .alerts()
            .add_update_written_elsewhere(id, AlertStatus::Fixing, &author)
            .await
            .expect("update");
    };
    post(serde_json::json!({"type": "from_a_newer_release"})).await;
    post(serde_json::json!({"type": "system", "job": "from_a_newer_release"})).await;
    let authors: Vec<_> = database
        .alerts()
        .updates(&[id])
        .await
        .expect("updates")
        .into_iter()
        .map(|(_, update)| update.author)
        .collect();
    assert_eq!(authors, [None, None]);

    post(serde_json::json!({"type": "cli"})).await;
    assert!(matches!(
        database.alerts().updates(&[id]).await,
        Err(DatabaseError::InvalidStoredData { .. })
    ));
}

/// An alert update's note is its event's reason, so it is read while the event's
/// personal data is kept, and reads as none once that is erased.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn an_alert_updates_note_erases_with_its_event() {
    use crate::repositories::AlertRepository;
    use sourceorigin_common::{
        AlertId,
        admin::{ChainBreak, ChainProblem},
        alerts::{Alert, AlertStatus},
    };
    let pool = Database::test_pool().await;
    let alert = Alert::AuditChainBroken(ChainBreak {
        position: 1,
        problem: ChainProblem::EventChanged,
    });
    let alert_id = AlertId::new();
    sqlx::query!(
        "INSERT INTO alerts (id, payload, version, cleared_at) VALUES ($1, $2, 1, now())",
        alert_id.as_uuid(),
        serde_json::to_value(&alert).expect("payload")
    )
    .execute(&pool)
    .await
    .expect("alert");
    let context = EventContext {
        actor: Actor::Cli {
            os_user: "ops".into(),
            host: "bastion".into(),
        },
        origin: RequestOrigin::default(),
        reason: Some("Restoring from backup".into()),
    };
    let updated = DomainEvent::AlertUpdated {
        alert_id,
        alert,
        status: AlertStatus::Fixing,
    };
    let mut tx = DatabaseTransaction::new(pool.begin().await.expect("transaction"));
    let event = tx.record(&context, None, updated).await.expect("record");
    tx.alerts()
        .add_update(alert_id, event, AlertStatus::Fixing, &context.actor)
        .await
        .expect("update");
    tx.commit().await.expect("commit");
    let note = || async {
        let posted = AlertRepository::from_pool(&pool)
            .updates(&[alert_id])
            .await
            .expect("updates");
        let [(_, update)] = <[_; 1]>::try_from(posted).expect("one update");
        update.note
    };
    assert_eq!(note().await.as_deref(), Some("Restoring from backup"));

    sqlx::query!(
        "DELETE FROM event_personal_data WHERE event_id = $1",
        event.as_uuid()
    )
    .execute(&pool)
    .await
    .expect("erase");
    assert_eq!(note().await, None);
}

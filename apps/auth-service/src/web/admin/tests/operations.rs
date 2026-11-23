//! The service's own lists: the audit chain, invitations, deliveries, signing
//! keys, handles and alerts.

use super::*;

/// The audit page's form verifies as `audit verify` does and comes back to the
/// page, which then shows that verification, and the page says that pruning is
/// the database owner's, from the command line. The database is shared, so other
/// tests may have verified already: the isolated audit tests show that loading the
/// page verifies nothing.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn the_audit_page_verifies_on_request_and_leaves_pruning_to_the_owner() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let dashboard = harness.sign_in(&admin, "/audit").await;
    let page = html(harness.visit(&dashboard, "/audit").await).await;
    assert!(page.contains("Audit history"), "{page}");
    assert!(page.contains("needs the database owner"), "{page}");
    let url = format!("{DASHBOARD}/audit");
    let verified = dashboard
        .submit(&harness.dashboard, &url, &[], OWN_PAGE)
        .await;
    assert_eq!(redirect(&verified), "/audit");
    let page = html(harness.visit(&dashboard, "/audit").await).await;
    assert!(page.contains("Events checked"), "{page}");
}

/// An invitation's link shows once, on the page creating it goes back to.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn an_invitations_link_shows_once() {
    let origin = AUTH_ORIGIN.parse().expect("auth origin");
    let mut config = AuthConfig::test(&origin);
    config.passkey.invite_required = true;
    let harness = Harness::of(config).await;
    let admin = harness.admin().await;
    let dashboard = harness
        .confirmed(&admin, "/invitations/new", Utc::now())
        .await;
    let created = dashboard
        .submit(
            &harness.dashboard,
            &format!("{DASHBOARD}/invitations/new"),
            &[("expires_in", "1d"), REASON],
            OWN_PAGE,
        )
        .await;
    assert_eq!(redirect(&created), "/invitations/new");
    let shown = harness
        .shown_once(&dashboard, &created, "invitation=", "/invitations")
        .await;
    assert!(
        shown.contains(&format!("{AUTH_ORIGIN}/register?")),
        "{shown}"
    );
}

/// Without email, the dashboard offers no mailing the service would refuse: a
/// mailed invitation can be revoked but not resent, and a new one asks for no
/// address, only how long its link lasts.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn invitations_offer_mailing_only_while_email_is_on() {
    let mut config = AuthConfig::test(&AUTH_ORIGIN.parse().expect("auth origin"));
    config.email.smtp = None;
    let harness = Harness::of(config).await;
    // Unique, since other scenarios share the database.
    let jane = format!("jane-{}@example.com", TokenId::new());
    // Mailed while email was on.
    let invitation = AdminService::new(harness.database.clone(), true)
        .create_invitation(
            &operator(),
            &crate::tests::invitation_links(),
            InviteExpiry::default(),
            Some(NormalizedEmail::parse(&jane).expect("address")),
            reason(),
        )
        .await
        .expect("invitation");
    let admin = harness.admin().await;
    let dashboard = harness
        .confirmed(&admin, "/invitations/new", Utc::now())
        .await;
    let page = html(harness.visit(&dashboard, "/invitations").await).await;
    let actions = format!("/invitations/{}", invitation.id);
    assert!(page.contains(&format!("{actions}/revoke")), "{page}");
    assert!(!page.contains(&format!("{actions}/resend")), "{page}");
    let form = html(harness.visit(&dashboard, "/invitations/new").await).await;
    assert!(form.contains(r#"name="expires_in""#), "{form}");
    assert!(!form.contains(r#"name="email""#), "{form}");
}

sourceorigin_repository::test_channels! {
    /// A delivery no worker sends, which the dashboard's tests list, cancel and retry.
    Unsent = "test_dashboard_unsent": () => ();
}

/// The deliveries page lists a channel's deliveries, filtered by status, each with
/// the change it may take: a pending one is cancelled from its row's dialog, and
/// then retried, each recorded as the administrator's: the dashboard acts as the
/// administrator signed in to it, never as the terminal.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn deliveries_are_cancelled_and_retried_from_the_dashboard() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let delivery = NewDelivery::<Unsent> {
        destination: "https://receiver.example".into(),
        recipient: (),
        payload: (),
    };
    let id = harness
        .database
        .deliveries()
        .insert(None, delivery)
        .await
        .expect("insert");
    let (cancel, retry) = (
        format!("/deliveries/{id}/cancel"),
        format!("/deliveries/{id}/retry"),
    );
    let dashboard = harness.confirmed(&admin, &cancel, Utc::now()).await;
    let listed = format!("/deliveries?status=&channel={}", Unsent::NAME);
    let page = html(harness.visit(&dashboard, &listed).await).await;
    assert!(page.contains(&format!(r#"action="{cancel}""#)), "{page}");
    assert!(
        !page.contains(&retry),
        "a pending delivery is not retried: {page}"
    );
    let failed = format!("/deliveries?status=failed&channel={}", Unsent::NAME);
    let page = html(harness.visit(&dashboard, &failed).await).await;
    assert!(page.contains("No delivery."), "{page}");
    let unknown = harness.visit(&dashboard, "/deliveries?status=lost").await;
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);

    let post = async |path: &str| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, &[REASON], OWN_PAGE)
            .await
    };
    assert_eq!(redirect(&post(&cancel).await), "/deliveries");
    let page = html(harness.visit(&dashboard, &listed).await).await;
    assert!(page.contains(&format!(r#"action="{retry}""#)), "{page}");
    assert!(page.contains("Cancelled"), "{page}");
    // Cancelled already, it is refused, saying why.
    let refused = html(post(&cancel).await).await;
    assert!(
        refused.contains("No pending or failed delivery"),
        "{refused}"
    );
    assert_eq!(redirect(&post(&retry).await), "/deliveries");
    for kind in ["delivery_cancelled", "delivery_retried"] {
        let actor = harness
            .actor_of(kind, |event| {
                matches!(
                    event,
                    DomainEvent::DeliveryCancelled { delivery_id }
                        | DomainEvent::DeliveryRetried { delivery_id } if *delivery_id == id
                )
            })
            .await;
        assert!(
            matches!(actor, Some(Actor::Admin { user_id, .. }) if user_id == admin.user_id),
            "{kind}: {actor:?}"
        );
    }
}

/// The signing keys page lists every key; an administrator publishes a next key
/// and revokes it.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn signing_keys_are_rotated_and_revoked_from_the_dashboard() {
    // Generating a key holds the signing key lock, which the replicas of other
    // tests on the shared database wait for as they start.
    let harness = Harness::isolated().await;
    let admin = harness.admin().await;
    let dashboard = harness.confirmed(&admin, "/keys/rotate", Utc::now()).await;
    let post = async |path: &str| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, &[REASON], OWN_PAGE)
            .await
    };
    assert_eq!(redirect(&post("/keys/rotate").await), "/keys");
    let next = harness
        .database
        .signing_keys()
        .list()
        .await
        .expect("keys")
        .into_iter()
        .find(|key| key.state == SigningKeyState::Next)
        .expect("a next key")
        .id;
    let revoke = format!("/keys/{next}/revoke");
    let page = html(harness.visit(&dashboard, "/keys").await).await;
    assert!(page.contains(&format!(r#"action="{revoke}""#)), "{page}");
    assert_eq!(redirect(&post(&revoke).await), "/keys");
    let keys = harness.database.signing_keys().list().await.expect("keys");
    assert!(keys.iter().all(|key| key.id != next), "revoked");
    let unknown = html(post(&revoke).await).await;
    assert!(unknown.contains("No signing key has ID"), "{unknown}");
}

/// The handles page manages every handle. An administrator reserves one for
/// someone, which the page lists and the rename form refuses until it allows a
/// reserved handle; the account holding it is then listed. Renamed again, its
/// old handle shows as released, and its row's release frees it; the
/// reservation's row removes it.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn handles_are_reserved_given_released_and_unreserved() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let user = owner.user_id;
    let suffix = &user.to_string()[24..];
    let kept = format!("kept-{suffix}");
    let rename = format!("/users/{user}/handle");
    let dashboard = harness
        .confirmed(&admin, "/handles/reserve", Utc::now())
        .await;
    let post = async |path: &str, fields: &[(&str, &str)]| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, fields, OWN_PAGE)
            .await
    };
    let page = html(harness.visit(&dashboard, "/handles").await).await;
    assert!(page.contains("<code>support</code>"), "built in: {page}");

    let reserve = [("handle", kept.as_str()), ("reason", "For the new CTO")];
    assert_eq!(
        redirect(&post("/handles/reserve", &reserve).await),
        "/handles"
    );
    let refused = html(post("/handles/reserve", &reserve).await).await;
    assert!(
        refused.contains("This handle, or one that looks like it, is reserved already"),
        "{refused}"
    );
    let unreserve = format!("/handles/unreserve?handle={kept}");
    let page = html(harness.visit(&dashboard, "/handles").await).await;
    assert!(page.contains("For the new CTO"), "{page}");
    assert!(page.contains(&format!(r#"action="{unreserve}""#)), "{page}");

    let page = html(harness.visit(&dashboard, &rename).await).await;
    assert!(page.contains(r#"name="allow_reserved""#), "{page}");
    let give = [("handle", kept.as_str()), ("reason", "Reserved for them")];
    let refused = html(post(&rename, &give).await).await;
    assert!(refused.contains("role=\"alert\""), "{refused}");
    assert!(
        refused.contains(&format!(r#"value="{kept}""#)),
        "kept: {refused}"
    );
    let allowed = [give.as_slice(), &[("allow_reserved", "on")]].concat();
    assert_eq!(
        redirect(&post(&rename, &allowed).await),
        format!("/users/{user}/profile")
    );
    let page = html(harness.visit(&dashboard, "/handles").await).await;
    assert!(page.contains(&rename), "the holder is listed: {page}");

    let back = format!("back-{suffix}");
    let renamed = [("handle", back.as_str()), ("reason", "Moved on")];
    assert_eq!(
        redirect(&post(&rename, &renamed).await),
        format!("/users/{user}/profile")
    );
    let release = format!("/handles/release?handle={kept}");
    let page = html(harness.visit(&dashboard, "/handles").await).await;
    assert!(
        page.contains(&format!(r#"action="{release}""#)),
        "released: {page}"
    );
    assert_eq!(redirect(&post(&release, &[REASON]).await), "/handles");
    let refused = html(post(&release, &[REASON]).await).await;
    assert!(refused.contains("This handle is not released"), "{refused}");
    assert_eq!(redirect(&post(&unreserve, &[REASON]).await), "/handles");
    let page = html(harness.visit(&dashboard, "/handles").await).await;
    assert!(
        !page.contains(&release) && !page.contains(&unreserve),
        "{page}"
    );
}

/// A change posted without a reason is refused on its page, which asks for one,
/// whatever else its form takes and however the page decodes it: reserving and
/// unreserving a handle, changing an account's handle and display name, and the
/// settings tabs, which read their form from the body themselves. None fails to
/// decode, and none changes anything.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_change_without_a_reason_is_refused_on_its_page() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let user = owner.user_id;
    let suffix = &user.to_string()[24..];
    let (kept, fresh) = (format!("kept-{suffix}"), format!("fresh-{suffix}"));
    let primary = format!("primary-{user}@example.com");
    crate::tests::verified_address(&harness.database, user, &primary, true).await;
    let address = format!("other-{user}@example.com");
    let other = crate::tests::verified_address(&harness.database, user, &address, false)
        .await
        .to_string();
    let stored = async || {
        harness
            .database
            .users()
            .find_by_id(user)
            .await
            .expect("lookup")
            .expect("user")
    };
    let before = stored().await;
    let dashboard = harness
        .confirmed(&admin, "/handles/reserve", Utc::now())
        .await;
    let post = async |path: &str, fields: &[(&str, &str)]| {
        let url = format!("{DASHBOARD}{path}");
        dashboard
            .submit(&harness.dashboard, &url, fields, OWN_PAGE)
            .await
    };
    let reserve = [("handle", kept.as_str()), REASON];
    assert_eq!(
        redirect(&post("/handles/reserve", &reserve).await),
        "/handles"
    );

    let unreserve = format!("/handles/unreserve?handle={kept}");
    let (handle, name, emails) = (
        format!("/users/{user}/handle"),
        format!("/users/{user}/name"),
        format!("/users/{user}/emails"),
    );
    let remove = [("action", "remove"), ("id", other.as_str())];
    for (path, fields) in [
        ("/handles/reserve", [("handle", fresh.as_str())].as_slice()),
        (&unreserve, &[]),
        (&handle, &[("handle", fresh.as_str())]),
        (&name, &[("display_name", "Renamed by support")]),
        (&emails, &remove),
    ] {
        let refused = post(path, fields).await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "{path}");
        let page = html(refused).await;
        assert!(
            page.contains("Give a reason of 1 to 500 characters"),
            "{path}: {page}"
        );
    }

    let page = html(harness.visit(&dashboard, "/handles").await).await;
    assert!(page.contains(&unreserve), "still reserved: {page}");
    assert!(!page.contains(&fresh), "nothing reserved: {page}");
    let after = stored().await;
    assert_eq!(after.handle, before.handle);
    assert_eq!(after.display_name, before.display_name);
    let addresses = harness.database.emails().list(user).await.expect("emails");
    assert!(
        addresses.iter().any(|email| email.id.to_string() == other),
        "the address stays"
    );
}

/// The Alerts page shows an open alert as needing attention, with posting an
/// update on it, and the overview's banner leads there. An update is recorded
/// as the administrator's, and the alert then shows its status and note. The
/// alerts that cleared are on a page of their own.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn alerts_take_updates_from_the_dashboard() {
    use sourceorigin_common::{
        admin::{ChainBreak, ChainProblem},
        alerts::{Alert, AlertKind},
        events::{EventContext, JobName},
    };
    let harness = Harness::isolated().await;
    let admin = harness.admin().await;
    // A position to find on the page.
    let position = Utc::now().timestamp_micros().unsigned_abs();
    let context = EventContext::system(JobName::AuditVerification);
    let mut tx = harness.database.begin().await.expect("transaction");
    let alert = Alert::AuditChainBroken(ChainBreak {
        position,
        problem: ChainProblem::EventChanged,
    });
    crate::alerts::Alerts::new(&mut tx, &context)
        .raise(alert)
        .await
        .expect("raise");
    let id = tx
        .alerts()
        .current(AlertKind::AuditChainBroken)
        .await
        .expect("current")
        .expect("raised")
        .id;
    tx.commit().await.expect("commit");
    let update = format!("/alerts/{id}/update");
    let dashboard = harness.confirmed(&admin, &update, Utc::now()).await;
    let page = html(harness.visit(&dashboard, "/alerts").await).await;
    assert!(page.contains(&format!(r#"action="{update}""#)), "{page}");
    assert!(page.contains(&position.to_string()), "{page}");
    assert!(page.contains("Needs attention"), "{page}");
    let overview = html(harness.visit(&dashboard, "/").await).await;
    assert!(
        overview.contains("1 alert needs attention") && overview.contains("See the alerts"),
        "{overview}"
    );

    let url = format!("{DASHBOARD}{update}");
    let fields = [("status", "fixing"), ("note", "Restoring from backup")];
    let done = dashboard
        .submit(&harness.dashboard, &url, &fields, OWN_PAGE)
        .await;
    assert_eq!(redirect(&done), "/alerts");
    let page = html(harness.visit(&dashboard, "/alerts").await).await;
    assert!(
        page.contains("Fixing") && page.contains("Restoring from backup"),
        "{page}"
    );
    let overview = html(harness.visit(&dashboard, "/").await).await;
    assert!(!overview.contains("See the alerts"), "{overview}");
    let actor = harness
        .actor_of(
            "alert_updated",
            |event| matches!(event, DomainEvent::AlertUpdated { alert_id, .. } if *alert_id == id),
        )
        .await;
    assert!(
        matches!(actor, Some(Actor::Admin { user_id, .. }) if user_id == admin.user_id),
        "{actor:?}"
    );
    let previous = html(harness.visit(&dashboard, "/alerts/previous").await).await;
    assert!(previous.contains("Previous alerts"), "{previous}");
    assert!(!previous.contains(&position.to_string()), "{previous}");
}

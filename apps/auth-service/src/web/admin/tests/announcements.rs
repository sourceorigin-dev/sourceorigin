//! Announcements: drafting, checking and sending them.

use super::*;
use crate::{AnnouncementDraft, AnnouncementRecipients, Letter, UserRef};
use sourceorigin_ui::{AnnouncementAudience, DraftAnnouncement};

/// The form lists one account per line, by handle, `@handle` as the dashboard
/// writes it, verified email or ID.
#[test]
fn the_form_names_one_account_per_line() {
    let id = UserId::new();
    let form = DraftAnnouncement {
        subject: "Incident".into(),
        body: "Text".into(),
        audience: AnnouncementAudience::Accounts,
        accounts: format!("ada\r\n\r\n  grace@example.com \n{id}\n@lin\n"),
        ..DraftAnnouncement::default()
    };
    let recipients = AnnouncementRecipients::Accounts(vec![
        UserRef::Handle("ada".into()),
        UserRef::Email("grace@example.com".into()),
        UserRef::Id(id),
        UserRef::Handle("lin".into()),
    ]);
    let letter = Letter::new("Incident", "Text").expect("a letter");
    assert_eq!(
        AnnouncementDraft::try_from(&form).expect("a draft"),
        AnnouncementDraft::new(letter, recipients, false).expect("a draft")
    );
}

/// The draft form finds accounts to add, saving nothing; each result adds itself
/// to the list, which also takes pasted accounts. "Send" saves the draft and
/// sends it as saved once the dashboard is confirmed, and otherwise goes on to the
/// send page, which confirms first and says it is essential.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn the_draft_form_finds_accounts_and_sends_from_the_editor() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let handle = harness
        .database
        .users()
        .find_by_id(owner.user_id)
        .await
        .expect("lookup")
        .expect("owner")
        .handle
        .to_string();
    let unconfirmed = harness.sign_in(&admin, "/announcements/new").await;
    let new = format!("{DASHBOARD}/announcements/new");
    let text = [("subject", "Hello group"), ("body", "Short note.")];
    let post = async |browser: &Browser, fields: &[(&str, &str)]| {
        let fields: Vec<_> = text.iter().chain(fields).copied().collect();
        browser
            .submit(&harness.dashboard, &new, &fields, OWN_PAGE)
            .await
    };
    let found = html(post(&unconfirmed, &[("intent", "find"), ("find", &handle)]).await).await;
    assert!(
        found.contains(&format!(r#"name="add" value="{handle}""#)),
        "{found}"
    );
    let adding = [
        ("find", handle.as_str()),
        ("accounts", "pasted"),
        ("add", &handle),
    ];
    let added = html(post(&unconfirmed, &adding).await).await;
    assert!(
        added.contains(&format!("pasted\n{handle}</textarea>")),
        "{added}"
    );
    assert!(added.contains("Added"), "{added}");
    let listed = harness.database.announcements().list(100).await;
    assert!(
        !listed
            .expect("list")
            .iter()
            .any(|sent| sent.subject == "Hello group"),
        "finding and adding save nothing"
    );

    let to = [
        ("audience", "accounts"),
        ("accounts", handle.as_str()),
        ("essential", "on"),
        ("intent", "send"),
    ];
    let unconfirmed_send = post(&unconfirmed, &to).await;
    let sending = redirect(&unconfirmed_send);
    assert!(sending.ends_with("/send?revision=1"), "{sending}");
    let confirmed = harness
        .confirmed(&admin, "/announcements/new", Utc::now())
        .await;
    let confirming = html(harness.visit(&confirmed, sending).await).await;
    assert!(
        confirming.contains("Yes: emailed whatever the recipient"),
        "{confirming}"
    );
    let sent = post(&confirmed, &to).await;
    let shown = html(harness.visit(&confirmed, redirect(&sent)).await).await;
    assert!(shown.contains("Sent"), "{shown}");
}

/// An administrator drafts an announcement without a fresh confirmation, and
/// its page shows the email as it will be sent. Sending needs the dashboard's
/// confirmation; once sent, the draft's pages lead back to it, and its recipient
/// reads it in their inbox.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn announcements_are_drafted_then_sent_from_the_dashboard() {
    let harness = Harness::new().await;
    let admin = harness.admin().await;
    let owner = Browser::new_account(&harness.database).await;
    let unconfirmed = harness.sign_in(&admin, "/announcements/new").await;
    let recipient = owner.user_id.to_string();
    let drafted = unconfirmed
        .submit(
            &harness.dashboard,
            &format!("{DASHBOARD}/announcements/new"),
            &[
                ("subject", "Planned maintenance"),
                (
                    "body",
                    "Sign-in pauses tonight.\r\n\r\nNothing else changes.",
                ),
                ("audience", "accounts"),
                ("accounts", &recipient),
            ],
            OWN_PAGE,
        )
        .await;
    let page = redirect(&drafted);
    assert!(page.starts_with("/announcements/"), "{page}");
    let draft = html(harness.visit(&unconfirmed, page).await).await;
    assert!(
        draft.contains("Hello,\n\nSign-in pauses tonight.\n\nNothing else changes."),
        "{draft}"
    );
    let send = format!("{page}/send?revision=1");
    assert!(draft.contains(&format!(r#"href="{send}""#)), "{draft}");
    let unconfirmed_send = harness.visit(&unconfirmed, &send).await;
    assert_eq!(location(&unconfirmed_send).path(), routes::page::ADMIN);

    let edited = unconfirmed
        .submit(
            &harness.dashboard,
            &format!("{DASHBOARD}{page}/edit"),
            &[
                ("subject", "Planned maintenance"),
                (
                    "body",
                    "Sign-in pauses tomorrow.\r\n\r\nNothing else changes.",
                ),
                ("audience", "accounts"),
                ("accounts", &recipient),
            ],
            OWN_PAGE,
        )
        .await;
    assert_eq!(redirect(&edited), page);
    let confirmed = harness.confirmed(&admin, &send, Utc::now()).await;
    let confirming = html(harness.visit(&confirmed, &send).await).await;
    assert!(
        confirming.contains(r#"<input type="hidden" name="revision" value="1">"#),
        "the page posts the revision it was opened for: {confirming}"
    );
    let post = async |revision: &str| {
        confirmed
            .submit(
                &harness.dashboard,
                &format!("{DASHBOARD}{page}/send?revision={revision}"),
                &[("revision", revision)],
                OWN_PAGE,
            )
            .await
    };
    let changed = html(post("1").await).await;
    assert!(
        changed.contains("draft changed since you checked it"),
        "{changed}"
    );
    let checked = html(harness.visit(&confirmed, page).await).await;
    assert!(checked.contains("Sign-in pauses tomorrow."), "{checked}");
    assert!(checked.contains("/send?revision=2"), "{checked}");
    assert_eq!(redirect(&post("2").await), page);
    let shown = html(harness.visit(&confirmed, page).await).await;
    assert!(!shown.contains("/send?revision="), "{shown}");
    for sent in [format!("{page}/edit"), format!("{page}/send?revision=2")] {
        assert_eq!(redirect(&harness.visit(&confirmed, &sent).await), page);
    }
    let inbox = html(
        owner
            .visit(&harness.auth.a, routes::page::settings::NOTIFICATIONS)
            .await,
    )
    .await;
    assert!(
        inbox.contains("<strong>Planned maintenance</strong>"),
        "{inbox}"
    );
    let mut notifications = harness.database.notifications();
    let received = notifications.list(owner.user_id, None, 10).await;
    let [received] = <[_; 1]>::try_from(received.expect("inbox")).expect("one notification");
    let item = routes::page::settings::NOTIFICATION.replace("{id}", &received.id.to_string());
    let shown = html(owner.visit(&harness.auth.a, &item).await).await;
    assert!(shown.contains("<p>Nothing else changes.</p>"), "{shown}");
}

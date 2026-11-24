//! What the overview recommends, and how.

use super::*;
use sourceorigin_common::{
    EmailId, PasskeyId,
    protocol::api::{Identity, Passkey},
};
use topcoat::{context::Cx, view::ViewExt};

/// The keys of what `overview` recommends, in order.
fn recommended(overview: &AccountOverview<'_>) -> Vec<String> {
    overview
        .recommendations()
        .into_iter()
        .map(Recommendation::key)
        .collect()
}

/// A new account lacks everything, most pressing first; an account holding
/// each lacks nothing. An unconfirmed address asks to be confirmed, and no
/// address is asked for while SourceOrigin sends no email.
#[test]
fn recommends_what_the_account_lacks_most_pressing_first() {
    let passkey = |name: &str| Passkey {
        friendly_name: name.to_owned(),
        id: PasskeyId::new(),
        created_at: Utc::now(),
    };
    let one = [passkey("Laptop")];
    let two = [passkey("Laptop"), passkey("Phone")];
    let pending = [EmailRow {
        id: EmailId::new(),
        address: "alice@example.com",
        verified: false,
        primary: false,
    }];
    let confirmed = [EmailRow {
        verified: true,
        primary: true,
        ..pending[0]
    }];
    let github = Identity {
        provider: Provider::GitHub,
        linked_email: None,
    };
    let unlinked = [
        ProviderRow {
            provider: Provider::GitHub,
            identity: None,
            enabled: true,
        },
        ProviderRow {
            provider: Provider::Google,
            identity: None,
            enabled: false,
        },
    ];
    let linked = [ProviderRow {
        identity: Some(&github),
        ..unlinked[0]
    }];
    let new = AccountOverview {
        devices: &[],
        apps: &[],
        app_sign_ins_week: 0,
        security_events_week: 0,
        activity: &[],
        holds: Holdings {
            passkeys: &one,
            ..Holdings::default()
        },
        providers: &unlinked,
        sends: true,
        confirmed: false,
        default_profile: true,
        now: Utc::now(),
        unread: 0,
    };
    assert_eq!(
        recommended(&new),
        [
            "sourceorigin-recommend:passkey",
            "sourceorigin-recommend:email",
            "sourceorigin-link:github",
            "sourceorigin-recommend:profile",
        ],
        "a disabled provider is never offered"
    );
    let held = AccountOverview {
        holds: Holdings {
            passkeys: &two,
            emails: &confirmed,
            ..Holdings::default()
        },
        providers: &linked,
        default_profile: false,
        ..new
    };
    assert_eq!(recommended(&held), Vec::<String>::new());
    let unconfirmed = AccountOverview {
        holds: Holdings {
            emails: &pending,
            ..held.holds
        },
        ..held
    };
    assert_eq!(
        recommended(&unconfirmed),
        ["sourceorigin-recommend:confirm-email"]
    );
    let silent = AccountOverview {
        sends: false,
        ..unconfirmed
    };
    assert_eq!(recommended(&silent), Vec::<String>::new());
}

/// A pending address's new link posts to the email addresses page, asking for a
/// passkey confirmation first unless the session was confirmed recently.
#[tokio::test]
async fn a_pending_address_is_sent_a_new_link_from_the_overview() {
    let pending = EmailRow {
        id: EmailId::new(),
        address: "alice@example.com",
        verified: false,
        primary: false,
    };
    let links = OverviewLinks {
        sign_in: "/settings/sign-in",
        emails: "/settings/emails",
        profile: "/settings/profile",
        devices: "/settings/sessions",
        history: "/settings/history",
        notifications: "/settings/notifications",
    };
    for confirmed in [false, true] {
        let cx = Cx::default();
        let hint = view! { cx =>
            recommendation_hint(
                recommendation: Recommendation::Email { pending: Some(&pending) },
                links: links,
                confirmed: confirmed,
            )
        }
        .single()
        .await
        .expect("the hint")
        .render(&cx);
        assert!(hint.contains(r#"action="/settings/emails""#), "{hint}");
        assert!(
            hint.contains(r#"aria-label="Resend link to alice@example.com""#),
            "{hint}"
        );
        assert_eq!(hint.contains("data-passkey-confirm"), !confirmed, "{hint}");
    }
}

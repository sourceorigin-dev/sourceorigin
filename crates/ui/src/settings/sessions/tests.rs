//! What a device's "Sign out" button is named.
use super::*;
use sourceorigin_common::events::{Browser, Platform};

/// Two devices of one name get Sign out buttons of different names, from
/// what their rows show: here, where they were last seen and when they
/// signed in.
#[test]
fn devices_of_one_name_are_signed_out_by_different_names() {
    let now = Utc::now();
    let firefox = DeviceRow {
        id: SessionId::new(),
        current: false,
        device: Some(UserAgentFamily {
            browser: Browser::Firefox,
            platform: Platform::Linux,
        }),
        ip: Some(IpAddr::from([192, 0, 2, 1])),
        methods: &[AuthenticationMethod::Passkey],
        authenticated_at: now - TimeDelta::days(3),
        last_seen_at: now,
        dashboard: &[],
    };
    let other = DeviceRow {
        id: SessionId::new(),
        ip: Some(IpAddr::from([192, 0, 2, 2])),
        authenticated_at: now - TimeDelta::days(1),
        ..firefox
    };
    let label = firefox.sign_out_label(now);
    assert!(
        label.starts_with("Sign out Firefox on Linux, Passkey, 192.0.2.1, "),
        "{label}"
    );
    assert!(!label.contains('·'), "{label}");
    assert_ne!(label, other.sign_out_label(now));
}

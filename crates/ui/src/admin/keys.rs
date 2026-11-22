//! The keys signing tokens.

use super::{Action, Badge, Change, badge, confirm, dashboard_change, data_table, moment};
use crate::{clients::choice, layout::page_head};
use chrono::{DateTime, Utc};
use sourceorigin_common::admin::{SigningKey, SigningKeyState};
use topcoat::{
    Result,
    view::{View, component, view},
};

/// A signing key, and the change revoking it.
#[derive(Clone, Debug)]
pub struct SigningKeyRow<'a> {
    /// The key.
    pub key: &'a SigningKey,
    /// Revoking it.
    pub revoke: Action,
}

/// A key's state, as a badge.
const fn state_badge(state: SigningKeyState) -> Badge {
    match state {
        SigningKeyState::Next => Badge::new("", "Waiting"),
        SigningKeyState::Active => Badge::new("ok", "Signing"),
        SigningKeyState::Retiring => Badge::new("neutral", "Retiring"),
        SigningKeyState::Retired => Badge::new("neutral", "Retired"),
    }
}

/// Every signing key, oldest first, `rows`, with the pages rotating them at
/// `rotate` and rewrapping them at `rewrap`.
#[component]
pub async fn signing_key_list<'a>(
    /// The signing keys, oldest first.
    rows: &'a [SigningKeyRow<'a>],
    /// Where the signing key is rotated.
    rotate: &'a str,
    /// Where the keys are rewrapped.
    rewrap: &'a str,
) -> Result<impl View> {
    Ok(view! {
        page_head(
            title: "Signing keys",
            lead: "The keys that sign tokens. A new key waits a day before it signs, so verifiers have it first, signs for 90 days, then stays published while its tokens may be valid. Every replica picks up a change within a minute.".to_owned(),
            actions: view! {
                <a data-confirm-first="" class="button secondary" href=(rewrap)>"Rewrap"</a>
                <a data-confirm-first="" class="button" href=(rotate)>"Rotate"</a>
            },
        )
        <section class="card">
            data_table(
                headings: &["key", "state", "created", "signed from", "signed until", "retired", ""],
                empty: "No signing key yet: auth generates one when it starts.",
                filled: !rows.is_empty(),
                for row in rows {
                    let key = row.key;
                    <tr>
                        <td><code>(key.id.as_str())</code> " " <small class="muted">(key.key_type.as_str())</small></td>
                        <td>badge(shown: state_badge(key.state))</td>
                        <td>moment(at: key.created_at)</td>
                        <td>when(at: key.activated_at)</td>
                        <td>when(at: key.deactivated_at)</td>
                        <td>when(at: key.retired_at)</td>
                        <td class="right">
                            confirm(action: &row.revoke, change: Change::RevokeSigningKey, subject: key.id.as_str())
                        </td>
                    </tr>
                }
            )
        </section>
    })
}

/// A moment, or a dash before it happens.
#[component]
async fn when(
    /// The moment, if it has happened.
    #[into]
    at: Option<DateTime<Utc>>,
) -> Result<impl View> {
    Ok(view! {
        match at {
            Some(at) => moment(at: at),
            None => <span class="muted">"—"</span>,
        }
    })
}

/// The page rotating the signing key, with why the last attempt was `refused`,
/// if it was, and whether it asked for the new key to sign `now`; it goes back
/// to `back`.
#[component]
pub async fn signing_key_rotation<'a>(
    /// Where the page goes back to.
    back: &'a str,
    #[into]
    #[default]
    /// Why the last attempt was refused, if it was.
    refused: Option<&'a str>,
    /// Whether the form asks for the new key to sign at once.
    now: bool,
    #[into]
    #[default]
    /// What the reason field holds.
    reason: Option<&'a str>,
) -> Result<impl View> {
    Ok(view! {
        dashboard_change(
            change: Change::RotateSigningKey,
            back: back,
            refusal: refused,
            reason: reason,
            choice(
                name: "now",
                checked: now,
                label: view! { "Sign with it at once" },
                detail: view! { "Skips the day of waiting: verifiers that cached the key set reject its tokens until they fetch it again, for up to an hour. For when the signing key must stop signing now; to stop trusting it, revoke it." },
            )
        )
    })
}

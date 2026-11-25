//! Email addresses, and confirming one.

use super::{Audience, Part};
use crate::{
    form::{Query, change_form, fragment_form, open},
    layout::{page_head, private_address},
};
use serde::{Deserialize, Serialize};
use sourceorigin_common::EmailId;
use topcoat::{
    Result,
    view::{View, component, view},
};

/// A change the addresses page posts, named by its `action` field. Each needs a
/// recent passkey confirmation. A field the user types is left empty here, and
/// the form renders its input instead.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum EmailChange {
    /// Adds an address, or sends a pending one a new link.
    Add {
        /// The address, which the user types unless it is resent.
        #[serde(skip_serializing_if = "String::is_empty")]
        email: String,
    },
    /// Makes a verified address the primary one.
    Primary {
        /// The address.
        id: EmailId,
    },
    /// Removes an address.
    Remove {
        /// The address.
        id: EmailId,
    },
}

/// Which row of the addresses is open, as the page's query names it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EmailsState {
    /// The address whose removal awaits confirmation.
    #[serde(rename = "remove")]
    pub removing: Option<EmailId>,
    /// The address an addition that failed asked for, to ask for it again.
    #[serde(skip)]
    pub adding: Option<String>,
    /// The address a refused change of the primary asked for, whose form alone
    /// keeps the reason typed.
    #[serde(skip)]
    pub making_primary: Option<EmailId>,
}

impl Query for EmailsState {}

/// One of the account's addresses.
#[derive(Clone, Copy, Debug)]
pub struct EmailRow<'a> {
    /// The address's ID.
    pub id: EmailId,
    /// The address.
    pub address: &'a str,
    /// Whether someone followed its verification link.
    pub verified: bool,
    /// Whether it is the primary address.
    pub primary: bool,
}

/// The email addresses page: the account's `emails`, primary first. Its rows open
/// on `page` as `state` says, and its changes post there, for `audience`. Only the
/// owner adds addresses, or sends one a new link, and only when SourceOrigin `sends`
/// email.
#[component]
pub async fn addresses<'a>(
    /// Where the page is, which its changes post to.
    page: &'a str,
    /// The account's addresses, primary first.
    emails: &'a [EmailRow<'a>],
    /// Which row is open, and what the last attempt said.
    state: &'a EmailsState,
    /// Whom the page is shown to.
    audience: Audience<'a>,
    /// Whether the service can send email.
    sends: bool,
) -> Result<impl View> {
    let replaceable = emails.iter().any(|email| email.verified && !email.primary);
    Ok(view! {
        if audience.owns() {
            page_head(
                title: "Email addresses",
                lead: "Origin Auth sends security notices to your primary address, and apps you sign in to receive it. Every address is verified before it is used.".to_owned(),
            )
        }
        <section class="card" aria-labelledby=(Part::Addresses.id())>
            <header>
                <h2 id=(Part::Addresses.id())>"Your addresses " <span class="count">(emails.len().to_string())</span></h2>
            </header>
            <ul class="list">
                for email in emails {
                    address_row(
                        page: page,
                        email: *email,
                        replaceable: replaceable,
                        state: state,
                        audience: audience,
                        sends: sends,
                    )
                }
            </ul>
            if emails.is_empty() {
                <p class="empty">"Add an address so SourceOrigin can tell you about changes to your account."</p>
            }
            if audience.owns() && sends {
                <details class="disclosure" id=(Part::AddAddress.id()) open=(state.adding.is_some())>
                    <summary><span class="button secondary small">"Add an address"</span></summary>
                    <div class="open-row">
                        change_form(
                            action: page,
                            change: EmailChange::Add { email: String::new() },
                            label: "Send a confirmation link",
                            confirm: audience.confirms(),
                            <label>
                                "Email address"
                                <input
                                    type="email"
                                    name="email"
                                    value=(state.adding.as_deref())
                                    autocomplete="email"
                                    maxlength="254"
                                    required=""
                                >
                            </label>
                        )
                    </div>
                </details>
            } else if audience.owns() {
                <p class="help">"Origin Auth cannot send email right now, so addresses cannot be added."</p>
            }
        </section>
    })
}

/// One address of [`addresses`]: its status and changes, and its removal row when
/// `state` opens it, which takes the focus. The primary address can be removed only
/// while no other verified one is `replaceable` with it, and an unverified one sent
/// a new link only when SourceOrigin `sends` email.
#[component]
async fn address_row<'a>(
    /// Where the page is, which its changes post to.
    page: &'a str,
    /// The address.
    email: EmailRow<'a>,
    /// Whether another verified address can replace it as primary.
    replaceable: bool,
    /// Which row is open, and what the last attempt said.
    state: &'a EmailsState,
    /// Whom the page is shown to.
    audience: Audience<'a>,
    /// Whether the service can send email.
    sends: bool,
) -> Result<impl View> {
    let id = email.id;
    let address = email.address;
    let blocked = email.primary && replaceable;
    Ok(view! {
        if state.removing == Some(id) {
            <li class="open">
                <p tabindex="-1" autofocus="">
                    (format!("Remove {address}?"))
                    if email.primary {
                        " SourceOrigin will no longer be able to email you security notices."
                    }
                </p>
                change_form(
                    action: page,
                    change: EmailChange::Remove { id },
                    label: "Confirm removal",
                    class: "danger",
                    confirm: audience.confirms(),
                    reason: audience.asks_reason(),
                    cancel: page.to_owned(),
                )
            </li>
        } else {
            <li>
                <div class="text">
                    <strong>private_address(address: address)</strong>
                    if email.primary {
                        " " <span class="badge">"Primary"</span>
                    } else if email.verified {
                        " " <span class="badge ok">"Verified"</span>
                    } else {
                        " " <span class="badge warning">"Waiting for confirmation"</span>
                    }
                    <small>
                        (if blocked {
                            "Receives security notices. Make another address primary to remove it."
                        } else if email.primary {
                            "Receives security notices"
                        } else if email.verified {
                            "Verified"
                        } else {
                            "Open the link we sent to confirm it"
                        })
                    </small>
                </div>
                if email.verified && !email.primary {
                    change_form(
                        action: page,
                        change: EmailChange::Primary { id },
                        label: "Make primary",
                        class: "secondary small",
                        aria_label: format!("Make primary {address}"),
                        confirm: audience.confirms(),
                        reason: if state.making_primary == Some(id) {
                            audience.asks_reason()
                        } else {
                            audience.asks_reason().map(|_| "")
                        },
                    )
                }
                if sends && !email.verified && audience.owns() {
                    change_form(
                        action: page,
                        change: EmailChange::Add { email: address.to_owned() },
                        label: "Resend link",
                        class: "secondary small",
                        aria_label: format!("Resend link to {address}"),
                        confirm: audience.confirms(),
                    )
                }
                open(
                    page: page,
                    opens: EmailsState { removing: Some(id), ..EmailsState::default() },
                    label: "Remove",
                    aria_label: format!("Remove {address}"),
                    disabled: blocked,
                    danger: true,
                )
            </li>
        }
    })
}

/// What the verification page says of a pasted link without its token.
pub const PASTE_WHOLE_LINK: &str =
    "Paste the whole link from the email, including everything after its #.";

/// Confirming an address from its verification link, which posts to `action`.
/// The link carries its token in the fragment, which never reaches a server: the
/// passkey script copies it into the form when it is posted, enabling the button
/// once it finds one, and shows instead that the link is incomplete when it has
/// none, as after a confirmation that failed, whose `alert` says why. Without
/// scripts, a second form asks for the link itself, pasted, whose token auth reads.
/// Its links lead to the addresses at `emails`.
#[component]
pub async fn verify<'a>(
    /// Where the form posts.
    action: &'a str,
    /// Where the addresses are listed.
    emails: &'a str,
    /// Why the last attempt failed, if it did.
    #[default]
    alert: Option<&'a str>,
) -> Result<impl View> {
    Ok(view! {
        <h1>"Confirm your email address"</h1>
        fragment_form(
            action: action,
            change: (),
            fragments: &["token"],
            label: "Confirm this address",
            unavailable: "Without scripts, paste the link from the email below instead.",
            alert: alert,
            missing: view! {
                <h2>"This link is incomplete"</h2>
                <p>"Open the link from the email again, or add the address again to get a new one."</p>
                <a class="button secondary" href=(emails)>"Manage your addresses"</a>
            },
            <p class="help">
                "Confirm only if you added this address to your own Origin Account: the email names the account it joins."
            </p>
        )
        <noscript>
            <form class="card stack" method="post" action=(action)>
                <label>
                    "Link from the email"
                    <input type="url" name="link" required="" autocomplete="off" spellcheck="false">
                </label>
                <p class="help">
                    "Copy the whole link from the email and paste it here. Confirm only if you added this address to your own Origin Account."
                </p>
                <button>"Confirm with this link"</button>
            </form>
        </noscript>
    })
}

/// The address now confirmed, and whether it became the `primary` one; its link
/// leads to the addresses at `emails`.
#[component]
pub async fn verified<'a>(
    /// The address.
    address: &'a str,
    /// Whether it became the primary one.
    primary: bool,
    /// Where the addresses are listed.
    emails: &'a str,
) -> Result<impl View> {
    Ok(view! {
        <h1>"Address confirmed"</h1>
        <p class="lead">
            (format!(
                "{address} is verified{}.",
                if primary { " and is now your primary address" } else { "" }
            ))
        </p>
        <a class="button" href=(emails)>"Manage your addresses"</a>
    })
}

//! The pages of OAuth and `OpenID` Connect flows: consent, logout, and why one
//! cannot go on.

use crate::{
    form::{Precision, Query, time},
    icons::{Icon, icon},
    layout::alert_notice,
    login::switch_account,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sourceorigin_common::{
    Provider,
    protocol::oauth::{ErrorCode, Scope},
};
use topcoat::{
    Result,
    view::{View, component, view},
};

/// The consent page: what the application named `client`, which sends the browser
/// back to `host`, asks to receive through `scopes`, and the decision on the
/// pending `transaction`, posted to `action` as `decision=approve` or
/// `decision=deny`. An application an account registered, `registered_by` by its
/// handle, is labelled unverified. The page names the signed-in `account`, and
/// offers to switch accounts by posting to `sign_in`, the sign-in page continuing
/// the transaction. The form posts once: a second decision would find the
/// authorization already used.
#[component]
pub async fn consent<'a>(
    /// What the application is called.
    client: &'a str,
    /// Where the browser is sent back to.
    host: &'a str,
    /// Who registered the application, if known.
    #[default]
    registered_by: Option<&'a str>,
    /// The account signed in.
    account: &'a str,
    /// What the application asks to receive.
    scopes: &'a [String],
    /// Where the decision posts.
    action: &'a str,
    /// Where to sign in as another account.
    sign_in: &'a str,
    /// The pending authorization.
    transaction: &'a str,
) -> Result<impl View> {
    Ok(view! {
        <div>
            <h1>(format!("Continue to {client}?"))</h1>
            <p class="lead">"You’ll be sent to " <strong>(host)</strong> " after you decide."</p>
        </div>
        if let Some(owner) = registered_by {
            <p class="notice warning">
                icon(kind: Icon::Warning)
                <span class="text">
                    (format!("Unverified app, registered by @{owner}"))
                    <small>"SourceOrigin hasn’t checked who runs it. Continue only if you trust it."</small>
                </span>
            </p>
        }
        <section class="card" aria-labelledby="access-heading">
            <header><h2 id="access-heading">(format!("{client} will receive"))</h2></header>
            <ul class="list">
                for scope in scopes {
                    <li>icon(kind: Icon::Check)<span class="text">(scope_label(scope))</span></li>
                }
            </ul>
        </section>
        <p class="help">
            (format!("{client} cannot see your passkeys or change your account. You can remove its access at any time from your connected apps."))
        </p>
        <form method="post" action=(action) data-submit-once="">
            <input type="hidden" name="transaction" value=(transaction)>
            <button name="decision" value="approve">"Continue"</button>
            <button class="secondary" name="decision" value="deny">"Cancel"</button>
        </form>
        <footer>
            <p class="help">(format!("Signed in as @{account}."))</p>
            switch_account(action: sign_in, label: "Not you? Switch account", class: "text-button")
        </footer>
    })
}

/// The device page, as a link opens it: with the code a device's link or QR code
/// carries, which the page fills in for the user to confirm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCodeQuery {
    /// The code the device shows, as its link wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_code: Option<String>,
}

impl Query for DeviceCodeQuery {}

sourceorigin_common::str_enum! {
    /// What the device page's user decides about the device asking to sign in.
    pub enum DeviceCodeDecision {
        /// Sign the device in.
        Approve = "approve",
        /// Refuse it.
        Deny = "deny",
    }
}

/// What the device page's forms post: the code, as typed or as the approval
/// carries it, and the decision once the user made one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct DeviceCodeForm {
    /// The code the device shows.
    #[serde(default)]
    pub user_code: String,
    /// The decision, from the approval's buttons; none while the user only typed
    /// the code.
    #[serde(default)]
    pub decision: Option<DeviceCodeDecision>,
}

/// The device page's first step: a form asking for the code a device shows,
/// starting with `code`, which posts to `action`. An `alert` says why the last
/// code was not accepted.
#[component]
pub async fn device_code<'a>(
    /// Where the code posts.
    action: &'a str,
    /// What the field starts with: the code of the device's link, or what was
    /// typed last.
    #[default]
    code: &'a str,
    /// Why the last code was not accepted, if one was typed.
    #[default]
    alert: Option<&'a str>,
) -> Result<impl View> {
    Ok(view! {
        <div>
            <h1>"Sign in on a device"</h1>
            <p class="lead">"Enter the code your device shows."</p>
        </div>
        alert_notice(message: alert)
        <form method="post" action=(action) data-submit-once="">
            <label>
                "Code"
                <input
                    class="device-code"
                    name="user_code"
                    value=(code)
                    placeholder="XXXX-XXXX"
                    required=""
                    maxlength="32"
                    autocomplete="off"
                    autocapitalize="characters"
                    spellcheck="false"
                    autofocus=(code.is_empty())
                >
            </label>
            <button>"Continue"</button>
        </form>
        <p class="help">
            "Only enter a code your own device shows you. If someone sent you a code or a link to this page, don’t use it: it would sign them in to your account."
        </p>
    })
}

/// The device page's approval: what the application named `client` asks to
/// receive through `scopes` on the device showing `code`, since `started`, how
/// long the device stays signed in, and the decision, posted to `action` with
/// the code. An application an account
/// registered, `registered_by` by its handle, is labelled unverified. The page
/// names the signed-in `account`, and offers to switch accounts by posting to
/// `sign_in`. The form posts once: a second decision would find the code used.
#[component]
pub async fn device_approval<'a>(
    /// What the application is called.
    client: &'a str,
    /// Who registered the application, if known.
    #[default]
    registered_by: Option<&'a str>,
    /// The account signed in.
    account: &'a str,
    /// What the application asks to receive.
    scopes: &'a [String],
    /// The code the device shows, written as devices show it.
    code: &'a str,
    /// When the device asked.
    started: DateTime<Utc>,
    /// How many days the device stays signed in without being used.
    days: u32,
    /// How many days the device stays signed in at most.
    longest_days: u16,
    /// Where the decision posts.
    action: &'a str,
    /// Where to sign in as another account.
    sign_in: &'a str,
) -> Result<impl View> {
    Ok(view! {
        <div>
            <h1>(format!("Sign in to {client} on your device?"))</h1>
            <p class="lead">
                "A device showing this code asked to sign in to your account at "
                time(at: started, precision: Precision::Minute) "."
            </p>
        </div>
        <p class="device-code">(code)</p>
        <p class="notice warning">
            icon(kind: Icon::Warning)
            <span class="text">
                "Only continue if you started this on your own device."
                <small>"If someone sent you this code or a link to this page, cancel: continuing would sign them in to your account."</small>
            </span>
        </p>
        if let Some(owner) = registered_by {
            <p class="notice warning">
                icon(kind: Icon::Warning)
                <span class="text">
                    (format!("Unverified app, registered by @{owner}"))
                    <small>"SourceOrigin hasn’t checked who runs it. Continue only if you trust it."</small>
                </span>
            </p>
        }
        <section class="card" aria-labelledby="access-heading">
            <header><h2 id="access-heading">(format!("{client} will receive"))</h2></header>
            <ul class="list">
                for scope in scopes {
                    <li>icon(kind: Icon::Check)<span class="text">(scope_label(scope))</span></li>
                }
            </ul>
        </section>
        <p class="help">
            (format!("{client} cannot see your passkeys or change your account. The device stays signed in while it is used at least every {days} days, for {longest_days} days at most: you can sign it out at any time from your devices."))
        </p>
        <form method="post" action=(action) data-submit-once="">
            <input type="hidden" name="user_code" value=(code)>
            <button name="decision" value=(DeviceCodeDecision::Approve.as_str())>"Continue"</button>
            <button class="secondary" name="decision" value=(DeviceCodeDecision::Deny.as_str())>"Cancel"</button>
        </form>
        <footer>
            <p class="help">(format!("Signed in as @{account}."))</p>
            switch_account(action: sign_in, label: "Not you? Switch account", class: "text-button")
        </footer>
    })
}

/// The device page's last step: what the user's `decision` did for the
/// application named `client`, and that they go back to their device.
#[component]
pub async fn device_decided<'a>(
    /// What the application is called.
    client: &'a str,
    /// What the user decided.
    decision: DeviceCodeDecision,
) -> Result<impl View> {
    let (title, lead) = match decision {
        DeviceCodeDecision::Approve => (
            "Your device is signed in".to_owned(),
            format!(
                "Go back to {client} on your device, which carries on in a few seconds. You can close this page."
            ),
        ),
        DeviceCodeDecision::Deny => (
            "Sign-in cancelled".to_owned(),
            format!("{client} was not signed in on the device. You can close this page."),
        ),
    };
    Ok(view! {
        <div>
            <h1>(title)</h1>
            <p class="lead">(lead)</p>
        </div>
    })
}

/// What a scope gives the application, in the user’s words, on the consent page
/// and among the connected apps. A scope SourceOrigin does not describe, such as an
/// application's own, is shown as it is.
pub(crate) fn scope_label(scope: &str) -> &str {
    match scope.parse() {
        Ok(Scope::OpenId) => "Your account identifier",
        Ok(Scope::Profile) => "Your username, display name and profile picture",
        Ok(Scope::Email) => "Your verified email address",
        Ok(Scope::OfflineAccess) => "Access your account while you’re away",
        Err(_) => scope,
    }
}

/// What the confirmation of an application's logout request warns about.
#[derive(Clone, Copy, Debug)]
pub enum LogoutNotice {
    /// Auth couldn't verify which sign-in the application asked to end, so the
    /// browser stays on auth afterwards.
    Unverified,
    /// The application asked to end another account's sign-in than this browser's.
    AnotherAccount,
}

impl LogoutNotice {
    /// What the page says.
    const fn text(self) -> &'static str {
        match self {
            Self::Unverified => {
                "Origin Auth couldn’t verify the app’s request, so you’ll stay here after signing out."
            }
            Self::AnotherAccount => {
                "The app asked to sign out a different account than the one signed in here. Signing out signs this browser out too."
            }
        }
    }
}

/// The confirmation of a logout: a form posting `fields`, the application's request,
/// if one asked, back to `action`, a link to `keep` the session, and any `notice`.
#[component]
pub async fn logout<'a>(
    /// Where the confirmation posts.
    action: &'a str,
    /// The application's request fields, sent back.
    fields: &'a [(&'a str, &'a str)],
    /// Where keeping the session goes.
    keep: &'a str,
    /// What the page says about the request, if anything.
    #[default]
    notice: Option<LogoutNotice>,
) -> Result<impl View> {
    Ok(view! {
        <div>
            <h1>"Ready to sign out?"</h1>
            <p class="lead">"This will end your Origin Account session in this browser."</p>
        </div>
        if let Some(notice) = notice {
            <p class="notice" role="status">(notice.text())</p>
        }
        <form method="post" action=(action)>
            for (name, value) in fields {
                <input type="hidden" name=(*name) value=(*value)>
            }
            <button>"Sign out"</button>
            <a class="button secondary" href=(keep)>"Keep my session"</a>
        </form>
    })
}

/// Why an OAuth page cannot go on: what `code` means for the user, the
/// `description` of what the request got wrong when they may see it, and a `link`
/// onwards as its label and URL.
#[component]
pub async fn failure<'a>(
    /// Why the flow cannot go on.
    code: &'a ErrorCode,
    /// What the request got wrong, when the user may see it.
    #[default]
    description: Option<&'a str>,
    /// Where the user may go on, with what to call it.
    #[default]
    link: Option<(&'a str, &'a str)>,
) -> Result<impl View> {
    let message = match code {
        ErrorCode::InvalidRequest => {
            "This request is invalid or expired. Start again from the application."
        }
        ErrorCode::InvalidToken | ErrorCode::LoginRequired => "Please sign in again.",
        ErrorCode::AccessDenied => "Sign-in was cancelled or access was declined.",
        ErrorCode::ConsentRequired => "Review the application’s requested access to continue.",
        ErrorCode::UnmetAuthenticationRequirements => {
            "This application needs a stronger sign-in than your account can give right now."
        }
        ErrorCode::ServerError => "Something went wrong on our side. Please try again.",
        ErrorCode::InvalidClient
        | ErrorCode::InvalidGrant
        | ErrorCode::UnauthorizedClient
        | ErrorCode::InvalidScope
        | ErrorCode::InsufficientScope
        | ErrorCode::UnsupportedResponseType
        | ErrorCode::InvalidTarget
        | ErrorCode::UnsupportedGrantType
        | ErrorCode::RequestNotSupported
        | ErrorCode::InvalidRequestUri
        | ErrorCode::AuthorizationPending
        | ErrorCode::SlowDown
        | ErrorCode::ExpiredToken
        | ErrorCode::Other(_) => {
            "This request could not be completed. Start again from the application."
        }
    };
    Ok(view! {
        <h1>(message)</h1>
        alert_notice(message: description)
        if let Some((label, href)) = link {
            <a class="button" href=(href)>(label)</a>
        }
    })
}

/// Where an [`onward`] link leads.
#[derive(Clone, Copy, Debug)]
pub enum Onward {
    /// Back to the application the flow is for.
    Application,
    /// To the provider, which signs the user in on the way to the application.
    Provider(Provider),
}

/// A step of a flow whose redirects a form may not lead through, because they end
/// at a destination no Content-Security-Policy can name, such as a native
/// application's loopback address: a link to `href`, which leads `to` there, that
/// the user follows instead.
#[component]
pub async fn onward(
    /// Where the step leads.
    href: &str,
    /// Where the link leads.
    to: Onward,
) -> Result<impl View> {
    let (title, lead) = match to {
        Onward::Application => (
            "Return to the application".to_owned(),
            "Continue to finish in the application.",
        ),
        Onward::Provider(provider) => (
            format!("Continue with {}", provider.name()),
            "Sign in there, then you return to the application.",
        ),
    };
    Ok(view! {
        <div>
            <h1>(title)</h1>
            <p class="lead">(lead)</p>
        </div>
        <a class="button" href=(href)>"Continue"</a>
    })
}

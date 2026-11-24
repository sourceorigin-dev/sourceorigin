//! The account at a glance: what is signed in and connected now, what happened
//! lately, and what the account still lacks.

use super::{
    ConnectedAppRow, DeviceRow, EmailChange, EmailRow, HistoryRow, Holdings, Part, ProviderRow,
    SignInChange, notifications::unread_hint, timeline::activity_feed,
};
use crate::{
    form::change_form,
    icons::{Icon, icon},
    layout::page_head,
};
use chrono::{DateTime, Utc};
use sourceorigin_common::Provider;
use topcoat::{
    Result,
    view::{View, component, view},
};

/// How many devices the overview lists.
const DEVICES_SHOWN: usize = 4;

/// How many recommendations the overview shows at once. Dismissing one shows the
/// next.
const RECOMMENDATIONS_SHOWN: usize = 2;

/// How many days a dismissed recommendation that protects the account stays
/// hidden before it shows again.
const RETURNS_AFTER_DAYS: u32 = 30;

/// The pages the overview links.
#[derive(Clone, Copy, Debug)]
pub struct OverviewLinks<'a> {
    /// The sign-in methods, whose form links a provider.
    pub sign_in: &'a str,
    /// The email addresses, whose form sends a pending address a new link.
    pub emails: &'a str,
    /// The profile.
    pub profile: &'a str,
    /// The devices.
    pub devices: &'a str,
    /// The security history.
    pub history: &'a str,
    /// The inbox.
    pub notifications: &'a str,
}

/// What the overview shows of the account.
#[derive(Clone, Copy, Debug)]
pub struct AccountOverview<'a> {
    /// Every browser and app signed in, most recently active first.
    pub devices: &'a [DeviceRow<'a>],
    /// The apps that keep access to the account.
    pub apps: &'a [ConnectedAppRow<'a>],
    /// How many times apps signed the account in over the last seven days.
    pub app_sign_ins_week: u64,
    /// How many security events the last seven days recorded.
    pub security_events_week: u64,
    /// The newest entries of the security history.
    pub activity: &'a [HistoryRow<'a>],
    /// What the account holds, to name what the entries refer to.
    pub holds: Holdings<'a>,
    /// The social sign-in providers, to offer one not yet linked.
    pub providers: &'a [ProviderRow<'a>],
    /// Whether SourceOrigin sends email, without which no address can be confirmed.
    pub sends: bool,
    /// Whether the session was confirmed with a passkey recently enough to send a
    /// pending address a new link without confirming again.
    pub confirmed: bool,
    /// Whether the profile keeps a default: no picture, or the handle as its
    /// display name.
    pub default_profile: bool,
    /// Now, to say when devices were active.
    pub now: DateTime<Utc>,
    /// How many notifications are unread.
    pub unread: u64,
}

impl<'a> AccountOverview<'a> {
    /// What the account still lacks, most pressing first.
    fn recommendations(&self) -> Vec<Recommendation<'a>> {
        let holds = &self.holds;
        let passkey = (holds.passkeys.len() == 1).then_some(Recommendation::Passkey);
        let email = (self.sends && !holds.emails.iter().any(|email| email.verified)).then(|| {
            Recommendation::Email {
                pending: holds.emails.first(),
            }
        });
        let providers = self
            .providers
            .iter()
            .filter(|row| row.enabled && row.identity.is_none())
            .map(|row| Recommendation::Provider(row.provider));
        let profile = self.default_profile.then_some(Recommendation::Profile);
        passkey
            .into_iter()
            .chain(email)
            .chain(providers)
            .chain(profile)
            .collect()
    }
}

/// Something the account still lacks, which the overview suggests until this
/// browser dismisses it.
#[derive(Clone, Copy, Debug)]
enum Recommendation<'a> {
    /// A second passkey, while the account has one: losing it takes a recovery
    /// through support.
    Passkey,
    /// A confirmed address, while the account has none: recovery links and
    /// security alerts go to it. `pending` is an address added but not yet
    /// confirmed, which the hint sends a new link.
    Email {
        /// The address awaiting confirmation, if any.
        pending: Option<&'a EmailRow<'a>>,
    },
    /// Linking a provider SourceOrigin offers, another way to sign in.
    Provider(Provider),
    /// A picture and a display name of the owner's choosing, which apps show.
    Profile,
}

impl Recommendation<'_> {
    /// The key a dismissal keeps in the browser. A provider's is kept from
    /// before the other recommendations, so its dismissal holds.
    fn key(self) -> String {
        match self {
            Self::Passkey => "sourceorigin-recommend:passkey".to_owned(),
            Self::Email { .. } => "sourceorigin-recommend:email".to_owned(),
            Self::Provider(provider) => format!("sourceorigin-link:{}", provider.as_str()),
            Self::Profile => "sourceorigin-recommend:profile".to_owned(),
        }
    }

    /// How many days a dismissal lasts, if not for good: those that protect the
    /// account come back.
    fn returns_after(self) -> Option<u32> {
        match self {
            Self::Passkey | Self::Email { .. } => Some(RETURNS_AFTER_DAYS),
            Self::Provider(_) | Self::Profile => None,
        }
    }

    /// Its icon.
    fn icon(self) -> Icon {
        match self {
            Self::Passkey => Icon::Key,
            Self::Email { .. } => Icon::Mail,
            Self::Provider(_) => Icon::Link,
            Self::Profile => Icon::Profile,
        }
    }

    /// What it suggests.
    fn title(self) -> String {
        match self {
            Self::Passkey => "Add a backup passkey".to_owned(),
            Self::Email { pending: None } => "Add an email address".to_owned(),
            Self::Email {
                pending: Some(email),
            } => format!("Confirm {}", email.address),
            Self::Provider(provider) => format!("You can still link {}", provider.name()),
            Self::Profile => "Finish your profile".to_owned(),
        }
    }

    /// Why.
    fn detail(self) -> String {
        match self {
            Self::Passkey => "If you lose your only passkey, getting back in takes a recovery through support. Keep a second one on another device or password manager.".to_owned(),
            Self::Email { pending: None } => "Recovery links and security alerts go to a confirmed address. Without one, they can't reach you.".to_owned(),
            Self::Email { pending: Some(_) } => "Open the link Origin Auth emailed to it. Until then, recovery links and security alerts can't reach you.".to_owned(),
            Self::Provider(provider) => format!(
                "Sign in with a {} account you already have. Linking never replaces your passkeys.",
                provider.name()
            ),
            Self::Profile => "Apps you sign in to show your display name and picture. Choose your own instead of your handle and initials.".to_owned(),
        }
    }
}

/// A [`Recommendation`] as a hint, hidden until the passkey script finds this
/// browser has not dismissed it, with its action on the pages in `links`. A
/// pending address's new link asks for a passkey confirmation unless the session
/// is `confirmed`.
#[component]
async fn recommendation_hint<'a>(
    recommendation: Recommendation<'a>,
    links: OverviewLinks<'a>,
    confirmed: bool,
) -> Result<impl View> {
    let key = recommendation.key();
    let days = recommendation.returns_after().map(|days| days.to_string());
    let anchored = |page: &str, part: Part| format!("{page}#{}", part.id());
    Ok(view! {
        <section class="hint" data-dismiss=(key.as_str()) data-dismiss-days=(days) hidden="">
            icon(kind: recommendation.icon())
            <div class="text">
                <strong>(recommendation.title())</strong>
                <small>(recommendation.detail())</small>
            </div>
            match recommendation {
                Recommendation::Passkey => {
                    <a class="button small" href=(anchored(links.sign_in, Part::AddPasskey))>"Add a passkey"</a>
                }
                Recommendation::Email { pending: None } => {
                    <a class="button small" href=(anchored(links.emails, Part::AddAddress))>"Add an address"</a>
                }
                Recommendation::Email { pending: Some(email) } => {
                    change_form(
                        action: links.emails,
                        change: EmailChange::Add { email: email.address.to_owned() },
                        label: "Resend link",
                        class: "small",
                        aria_label: format!("Resend link to {}", email.address),
                        confirm: !confirmed,
                    )
                }
                Recommendation::Provider(provider) => {
                    change_form(
                        action: links.sign_in,
                        change: SignInChange::Link { provider },
                        label: format!("Link {}", provider.name()),
                        class: "small",
                    )
                }
                Recommendation::Profile => {
                    <a class="button small" href=(links.profile)>"Edit profile"</a>
                }
            }
            <button type="button" class="secondary small" data-dismisses=(key.as_str())>"Not now"</button>
        </section>
    })
}

/// The overview page: what is unread in the inbox, the tiles, two at a time of
/// what the account still lacks, which the page offers until the browser
/// dismisses them, the newest security activity and the devices signed in,
/// linking the pages in `links`.
#[component]
pub async fn account_overview<'a>(
    /// What the page shows.
    overview: AccountOverview<'a>,
    /// Where each tile leads.
    links: OverviewLinks<'a>,
) -> Result<impl View> {
    let devices = overview.devices;
    let (browsers, other) = devices.iter().fold((0, 0), |(browsers, other), device| {
        if device.device.is_some() {
            (browsers + 1, other)
        } else {
            (browsers, other + 1)
        }
    });
    let names: Vec<_> = overview.apps.iter().map(|app| app.name).collect();
    let recommendations = overview.recommendations();
    Ok(view! {
        page_head(title: "Overview", lead: "Your account at a glance.".to_owned())
        unread_hint(unread: overview.unread, page: links.notifications)
        <div class="stats">
            <div class="stat">
                <span class="label">"Active sessions"</span>
                <span class="value">(devices.len().to_string())</span>
                <span class="detail">(sessions_detail(browsers, other))</span>
            </div>
            <div class="stat">
                <span class="label">"Connected apps"</span>
                <span class="value">(overview.apps.len().to_string())</span>
                <span class="detail">(if names.is_empty() { "none yet".to_owned() } else { names.join(" · ") })</span>
            </div>
            <div class="stat">
                <span class="label">"App sign-ins, 7 days"</span>
                <span class="value">(overview.app_sign_ins_week.to_string())</span>
                <span class="detail">"through apps you use"</span>
            </div>
            <div class="stat">
                <span class="label">"Security events, 7 days"</span>
                <span class="value">(overview.security_events_week.to_string())</span>
                <span class="detail">"sign-ins and changes"</span>
            </div>
        </div>
        <div class="hints" data-dismiss-limit=(RECOMMENDATIONS_SHOWN.to_string())>
            for recommendation in recommendations {
                recommendation_hint(
                    recommendation: recommendation,
                    links: links,
                    confirmed: overview.confirmed,
                )
            }
        </div>
        <div class="two side">
            <section class="card" aria-labelledby=(Part::Activity.id())>
                <header>
                    <h2 id=(Part::Activity.id())>"Recent security activity"</h2>
                    <a href=(links.history)>"See all history"</a>
                </header>
                if overview.activity.is_empty() {
                    <p class="empty">"Nothing has happened to your account yet."</p>
                } else {
                    activity_feed(rows: overview.activity, holds: &overview.holds)
                }
            </section>
            <section class="card" aria-label="Devices">
                <header>
                    <h2>"Devices"</h2>
                    <a href=(links.devices)>"Manage"</a>
                </header>
                <ul class="list">
                    for device in devices.iter().take(DEVICES_SHOWN) {
                        <li>
                            <div class="text">
                                <strong>(device.name())</strong>
                                if device.current {
                                    " " <span class="badge ok">"This device"</span>
                                }
                                <small>(device.details(overview.now).join(" · "))</small>
                            </div>
                        </li>
                    }
                </ul>
                if devices.len() > DEVICES_SHOWN {
                    <p class="help">
                        <a href=(links.devices)>(format!("And {} more", devices.len() - DEVICES_SHOWN))</a>
                    </p>
                }
            </section>
        </div>
    })
}

/// How many of the sessions are recorded `browsers`, and how many `other`
/// devices went unrecorded, in words.
fn sessions_detail(browsers: usize, other: usize) -> String {
    let plural =
        |count: usize, word: &str| format!("{count} {word}{}", if count == 1 { "" } else { "s" });
    match (browsers, other) {
        (0, 0) => "none".to_owned(),
        (browsers, 0) => plural(browsers, "browser"),
        (0, other) => plural(other, "unrecorded device"),
        (browsers, other) => format!("{} · {other} unrecorded", plural(browsers, "browser")),
    }
}

#[cfg(test)]
mod tests {
    //! What the overview recommends.
    use super::*;
    use sourceorigin_common::{
        EmailId, PasskeyId,
        protocol::api::{Identity, Passkey},
    };

    /// The keys of what `overview` recommends, in order, with a pending
    /// address's title.
    fn recommended(overview: &AccountOverview<'_>) -> Vec<String> {
        overview
            .recommendations()
            .into_iter()
            .map(|recommendation| match recommendation {
                Recommendation::Email { pending: Some(_) } => recommendation.title(),
                _ => recommendation.key(),
            })
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
        assert_eq!(recommended(&unconfirmed), ["Confirm alice@example.com"]);
        let silent = AccountOverview {
            sends: false,
            ..unconfirmed
        };
        assert_eq!(recommended(&silent), Vec::<String>::new());
    }
}

//! An OIDC client as the pages managing it show it: its status, its facts, the
//! form of its settings, its secrets and the one just made. The developer console
//! shows an account's own apps with these, and the admin dashboard every client.

use crate::{
    form::{Checkbox, Fields, Lines, Precision, change_form, time},
    layout::alert_notice,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sourceorigin_common::{
    ClientId, MAX_NAME_CHARS,
    admin::{ClientDetail, SecretLifetime},
    clients::{ClientField, ClientKind, ClientStatus},
    protocol::oauth::{GrantType, Scope},
};
use std::{borrow::Cow, fmt};
use topcoat::{
    Result,
    view::{Child, View, component, view},
};

/// A client, as the pages show it.
#[derive(Clone, Copy, Debug)]
pub struct ClientRow<'a> {
    /// Its ID.
    pub id: &'a ClientId,
    /// The name consent screens show.
    pub name: &'a str,
    /// Whether it keeps a secret.
    pub kind: ClientKind,
    /// Whether it signs users in.
    pub status: ClientStatus,
    /// Its authorization callbacks.
    pub redirect_uris: &'a [String],
    /// The pages the browser may return to after logout.
    pub post_logout_uris: &'a [String],
    /// The scopes it may request.
    pub allowed_scopes: &'a [String],
    /// The grants it may use at the token endpoint.
    pub grant_types: &'a [GrantType],
    /// Whether it must push its authorization requests.
    pub require_pushed_authorization_requests: bool,
    /// Where logout tokens are posted, if anywhere.
    pub backchannel_logout_uri: Option<&'a str>,
    /// When a token was last issued to it, if ever.
    pub last_used_at: Option<DateTime<Utc>>,
}

impl<'a> From<&'a ClientDetail> for ClientRow<'a> {
    /// The client as the dashboard reads it.
    fn from(detail: &'a ClientDetail) -> Self {
        let client = &detail.client;
        Self {
            id: &client.id,
            name: &client.display_name,
            kind: detail.kind,
            status: client.status(),
            redirect_uris: &client.redirect_uris,
            post_logout_uris: &client.post_logout_uris,
            allowed_scopes: &client.allowed_scopes,
            grant_types: &client.grant_types,
            require_pushed_authorization_requests: client.require_pushed_authorization_requests,
            backchannel_logout_uri: client.backchannel_logout_uri.as_deref(),
            last_used_at: client.last_used_at,
        }
    }
}

impl ClientRow<'_> {
    /// What its kind means for its owner.
    pub(crate) const fn kind_name(&self) -> &'static str {
        match self.kind {
            ClientKind::Confidential => "Web app with a backend",
            ClientKind::Public => "Browser or native app",
        }
    }
}

/// One of a confidential client's secrets: when it was made and when it stops
/// working, never its value.
#[derive(Clone, Copy, Debug)]
pub struct SecretRow {
    /// When it was made.
    pub created_at: DateTime<Utc>,
    /// When a rotation made it stop working, if one did.
    pub expires_at: Option<DateTime<Utc>>,
}

impl From<&SecretLifetime> for SecretRow {
    /// The secret's lifetime, as the dashboard reads it.
    fn from(secret: &SecretLifetime) -> Self {
        Self {
            created_at: secret.created_at,
            expires_at: secret.expires_at,
        }
    }
}

/// Where a client's `OpenID` Connect library finds Origin Auth.
#[derive(Clone, Copy, Debug)]
pub struct Connection<'a> {
    /// The issuer.
    pub issuer: &'a str,
    /// Its discovery document.
    pub discovery: &'a str,
}

/// A secret Origin Auth just generated, shown only in the response to the change that
/// made it. It never serializes and its `Debug` is redacted, so no query, link or
/// log can hold it.
pub struct Revealed(String);

impl Revealed {
    /// Shows `secret`.
    #[must_use]
    pub const fn new(secret: String) -> Self {
        Self(secret)
    }

    /// The secret.
    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Revealed {
    /// Writes nothing of the secret.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Revealed(REDACTED)")
    }
}

sourceorigin_common::str_enum! {
    /// How long a client's earlier secrets keep working once it gets a new one, as
    /// the rotation form offers it, written as auth reads grace periods.
    pub enum Grace {
        /// They stop at once.
        Now = "0",
        /// For an hour.
        Hour = "1h",
        /// For a day.
        Day = "24h",
        /// For a week.
        Week = "7d",
        /// For 30 days.
        Month = "30d",
    }
}

impl Grace {
    /// The choice in words.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Now => "Stop them now",
            Self::Hour => "For an hour",
            Self::Day => "For a day",
            Self::Week => "For a week",
            Self::Month => "For 30 days",
        }
    }
}

impl Default for Grace {
    /// A day, which the rotation form's select starts at.
    fn default() -> Self {
        Self::Day
    }
}

/// Declares the switches a client's form offers for a closed vocabulary, one
/// checkbox per value, named as its field is: a form sends a list as repeated
/// names, which a change cannot decode.
macro_rules! switches {
    (
        $(#[$meta:meta])*
        $name:ident of $vocabulary:ident {
            $($(#[$field_meta:meta])* $field:ident => $variant:ident),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
        pub struct $name {
            $(
                $(#[$field_meta])*
                #[serde(default, skip_serializing_if = "Checkbox::is_off")]
                pub $field: Checkbox,
            )+
        }

        impl $name {
            /// The checkbox of `value`. Exhaustive, so a new value does not compile
            /// until the form offers it.
            const fn checkbox(&mut self, value: $vocabulary) -> &mut Checkbox {
                match value {
                    $($vocabulary::$variant => &mut self.$field,)+
                }
            }

            /// Whether `value` is chosen.
            const fn has(mut self, value: $vocabulary) -> bool {
                self.checkbox(value).0
            }

            /// The values chosen, in the vocabulary's order.
            pub fn chosen(self) -> impl Iterator<Item = $vocabulary> {
                $vocabulary::ALL
                    .into_iter()
                    .filter(move |value| self.has(*value))
            }
        }

        impl FromIterator<$vocabulary> for $name {
            /// The given values chosen.
            fn from_iter<I: IntoIterator<Item = $vocabulary>>(values: I) -> Self {
                let mut chosen = Self::default();
                for value in values {
                    *chosen.checkbox(value) = Checkbox(true);
                }
                chosen
            }
        }
    };
}

switches! {
    /// The standard scopes a client's form chose, each switch named after its scope.
    ScopeChoice of Scope {
        /// `openid`.
        openid => OpenId,
        /// `profile`.
        profile => Profile,
        /// `email`.
        email => Email,
        /// `offline_access`.
        offline_access => OfflineAccess,
    }
}

switches! {
    /// The grant types a client's form chose, each switch named after its grant.
    GrantChoice of GrantType {
        /// `authorization_code`.
        authorization_code => AuthorizationCode,
        /// `refresh_token`.
        refresh_token => RefreshToken,
        /// The device grant, under its URN.
        #[serde(rename = "urn:ietf:params:oauth:grant-type:device_code")]
        device_code => DeviceCode,
        /// `client_credentials`.
        client_credentials => ClientCredentials,
    }
}

/// A client's settings as its form sends them. A form leaves every setting empty
/// in the change it sends as hidden inputs, and renders the inputs of the draft it
/// shows instead. Browsers send every text input and textarea; a textarea left out
/// reads as empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientDraft {
    /// The name consent screens show.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    /// Its authorization callbacks.
    #[serde(default, skip_serializing_if = "Lines::is_empty")]
    pub redirect_uris: Lines,
    /// The pages the browser may return to after logout.
    #[serde(default, skip_serializing_if = "Lines::is_empty")]
    pub post_logout_uris: Lines,
    /// The scopes it may request.
    #[serde(flatten)]
    pub scopes: ScopeChoice,
    /// The grants it may use at the token endpoint.
    #[serde(flatten)]
    pub grants: GrantChoice,
    /// Whether it must push its authorization requests.
    #[serde(default, skip_serializing_if = "Checkbox::is_off")]
    pub require_pushed_authorization_requests: Checkbox,
    /// Where auth posts its logout tokens; empty for nowhere.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub backchannel_logout_uri: String,
}

impl ClientDraft {
    /// The settings a new client of `kind` starts with: its default scopes and
    /// grant types.
    #[must_use]
    pub fn new_client(kind: ClientKind) -> Self {
        Self {
            scopes: kind.default_scopes().iter().copied().collect(),
            grants: kind.default_grant_types().iter().copied().collect(),
            ..Self::default()
        }
    }

    /// The names of the scope and grant type switches it turns on, space-separated.
    fn switches(&self) -> String {
        let scopes = self.scopes.chosen().map(Scope::as_str);
        let grants = self.grants.chosen().map(GrantType::as_str);
        scopes.chain(grants).collect::<Vec<_>>().join(" ")
    }
}

impl From<&ClientRow<'_>> for ClientDraft {
    /// `client`'s settings as they are. Only standard scopes have a checkbox.
    fn from(client: &ClientRow<'_>) -> Self {
        Self {
            display_name: client.name.to_owned(),
            redirect_uris: client.redirect_uris.iter().cloned().collect(),
            post_logout_uris: client.post_logout_uris.iter().cloned().collect(),
            scopes: client
                .allowed_scopes
                .iter()
                .filter_map(|scope| scope.parse().ok())
                .collect(),
            grants: client.grant_types.iter().copied().collect(),
            require_pushed_authorization_requests: Checkbox(
                client.require_pushed_authorization_requests,
            ),
            backchannel_logout_uri: client.backchannel_logout_uri.unwrap_or_default().to_owned(),
        }
    }
}

/// Why the service refused one of a client's settings, as its form shows it.
#[derive(Clone, Copy, Debug)]
pub struct FieldError<'a> {
    /// The setting.
    pub field: &'a ClientField,
    /// What is wrong with it, without its name.
    pub message: &'a str,
}

impl FieldError<'_> {
    /// Whether the client form shows a refusal of its setting beside the setting's
    /// input. Only the settings the service validates have a place for one: it
    /// never refuses the pushed-request checkbox.
    const fn placed(&self) -> bool {
        match self.field {
            ClientField::DisplayName
            | ClientField::RedirectUris
            | ClientField::PostLogoutUris
            | ClientField::AllowedScopes
            | ClientField::GrantTypes
            | ClientField::BackchannelLogoutUri => true,
            ClientField::Trusted
            | ClientField::RequirePushedAuthorizationRequests
            | ClientField::Other(_) => false,
        }
    }
}

/// One of a client's form's inputs, and what the form says of it in elements the
/// input names, whose IDs start with its own: its help, and why the service
/// refused its setting, if it did.
struct Described<'a> {
    /// The input's ID.
    id: String,
    /// Why the service refused its setting, if it did.
    refusal: Option<&'a str>,
}

impl<'a> Described<'a> {
    /// The input `id`, whose setting the service did not refuse.
    const fn new(id: String) -> Self {
        Self { id, refusal: None }
    }

    /// The input of `field`, named after it, and `error`'s refusal of it, if it
    /// is about it.
    fn of(error: Option<FieldError<'a>>, field: &ClientField) -> Self {
        Self {
            id: field.to_string(),
            refusal: error
                .filter(|error| error.field == field)
                .map(|error| error.message),
        }
    }

    /// The ID of its help.
    fn help(&self) -> String {
        format!("{}-help", self.id)
    }

    /// The ID of the element saying why its setting was refused.
    fn error(&self) -> String {
        format!("{}-error", self.id)
    }

    /// Its `aria-invalid`.
    fn flag(&self) -> Option<&'static str> {
        self.refusal.map(|_| "true")
    }

    /// Its `aria-describedby`: its help, then why its setting was refused, if it
    /// was.
    fn described_by(&self) -> String {
        if self.refused() {
            format!("{} {}", self.help(), self.error())
        } else {
            self.help()
        }
    }

    /// Whether its setting was refused, so it takes the focus.
    const fn refused(&self) -> bool {
        self.refusal.is_some()
    }
}

/// What a standard scope gives a client.
const fn scope_description(scope: Scope) -> &'static str {
    match scope {
        Scope::OpenId => "Sign users in: their stable user ID",
        Scope::Profile => "Their handle, display name and profile picture",
        Scope::Email => "Their email address, when they have one",
        Scope::OfflineAccess => "Refresh tokens, with the refresh token grant",
    }
}

/// What a grant type lets a client do, under the name its switch shows.
const fn grant_description(grant: GrantType) -> (&'static str, &'static str) {
    match grant {
        GrantType::AuthorizationCode => (
            "Authorization code",
            "Users sign in on Origin Auth, which sends them back to a redirect URI with a code your app exchanges.",
        ),
        GrantType::RefreshToken => (
            "Refresh tokens",
            "When your app requests offline_access, it can stay signed in without sending users back to Origin Auth.",
        ),
        GrantType::DeviceCode => (
            "Device code",
            "Your app shows a code on a device without a browser, such as a command-line tool or a TV, and users approve it on Origin Auth from another device.",
        ),
        GrantType::ClientCredentials => (
            "Client credentials",
            "Your backend gets tokens for itself, with its secret and no user, for the APIs that gave it access. Only for an app with a backend.",
        ),
    }
}

/// A client's kind, then a warning when it does not sign users in, as badges.
#[component]
pub(crate) async fn client_status<'a>(
    /// The client.
    client: ClientRow<'a>,
) -> Result<impl View> {
    Ok(view! {
        <span class="badge neutral">(client.kind_name())</span>
        match client.status {
            ClientStatus::Deleted { .. } => {
                " " <span class="badge neutral">"Deleted"</span>
            },
            ClientStatus::Suspended { .. } => {
                " " <span class="badge danger">"Suspended"</span>
            },
            ClientStatus::Disabled => {
                " " <span class="badge warning">"Disabled"</span>
            },
            ClientStatus::Enabled => {},
        }
    })
}

/// A button copying the text of the element whose ID is `target`, named `label` for
/// screen readers, after the status saying whether it did. It starts hidden; the
/// copy script shows it once it can write to the clipboard.
#[component]
pub(crate) async fn copy<'a>(
    /// The ID of the element whose text is copied.
    target: &'a str,
    /// What the button is named for screen readers.
    label: &'a str,
) -> Result<impl View> {
    Ok(view! {
        <span class="copy">
            <span class="help" role="status"></span>
            <button type="button" class="text-button small" data-copy=(target) aria-label=(format!("Copy {label}")) hidden="">
                "Copy"
            </button>
        </span>
    })
}

/// `value` as code, the element `id`, beside a button copying it, named `label`.
/// Clicking the code copies it too.
#[component]
pub(crate) async fn copyable<'a>(
    /// The code element's ID.
    id: &'a str,
    /// What the copy button is named for screen readers.
    label: &'a str,
    /// The text shown and copied.
    value: &'a str,
) -> Result<impl View> {
    Ok(view! {
        <div class="code-line">
            <code id=(id)>(value)</code>
            copy(target: id, label: label)
        </div>
    })
}

/// What a client's `OpenID` Connect library needs to know, found through
/// `connection`, each value with a button copying it, then its redirect URIs.
#[component]
pub(crate) async fn client_facts<'a>(
    /// The client.
    client: ClientRow<'a>,
    /// Where a client library finds the service.
    connection: Connection<'a>,
) -> Result<impl View> {
    let mut facts: Vec<(&str, &str, Cow<'_, str>)> = vec![
        ("issuer", "Issuer", connection.issuer.into()),
        (
            "discovery-url",
            "Discovery URL",
            connection.discovery.into(),
        ),
        ("client-id", "Client ID", client.id.as_str().into()),
        (
            "client-auth",
            "Client authentication",
            client.kind.auth_method().as_str().into(),
        ),
        ("scopes", "Scopes", client.allowed_scopes.join(" ").into()),
    ];
    facts.extend(
        client
            .backchannel_logout_uri
            .map(|uri| ("backchannel-logout", "Back-channel logout URI", uri.into())),
    );
    Ok(view! {
        <dl class="facts">
            for (id, label, value) in facts {
                <dt>(label)</dt>
                <dd>copyable(id: id, label: label, value: &value)</dd>
            }
            <dt>"Redirect URIs"</dt>
            <dd>
                for uri in client.redirect_uris {
                    <div><code>(uri.as_str())</code></div>
                }
                if client.redirect_uris.is_empty() { "None" }
            </dd>
        </dl>
    })
}

/// Help with the input `of`, below it.
#[component]
async fn help<'a>(
    /// The input described.
    of: &'a Described<'a>,
    /// The help text.
    child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <small class="field-help" id=(of.help())>(child)</small>
    })
}

/// Why the service refused the setting of the input `of`, if it did, below it.
#[component]
async fn refusal<'a>(
    /// The input whose setting was refused.
    of: &'a Described<'a>,
) -> Result<impl View> {
    Ok(view! {
        if let Some(message) = of.refusal {
            <p class="field-error" id=(of.error())>(message)</p>
        }
    })
}

/// A switch sending `name` when on, or with a `value`, a radio button of the
/// group `name` choosing it, `checked` or not, which takes the focus if
/// `autofocus`: its `label` beside it, then its `detail` below, which describes
/// it. A radio button's `defaults` name the switches choosing it turns on, for
/// the client form script. The switch is a checkbox with the `switch` role,
/// which the stylesheet draws as a sliding toggle at the right end of a boxed
/// row; switches inside a `choices` element share one box.
#[component]
pub(crate) async fn choice<'a>(
    /// The name the switch or the radio group posts under.
    name: &'a str,
    /// What a radio button posts; none makes a switch.
    #[default]
    value: Option<&'a str>,
    /// Whether the choice is on.
    checked: bool,
    /// Whether the choice starts focused.
    #[default]
    autofocus: bool,
    /// The switches choosing a radio button turns on.
    #[default]
    defaults: Option<String>,
    /// What the choice is called.
    #[into]
    label: Child<'_>,
    /// What the choice does.
    #[into]
    detail: Child<'_>,
) -> Result<impl View> {
    let input =
        Described::new(value.map_or_else(|| name.to_owned(), |value| format!("{name}-{value}")));
    Ok(view! {
        <div class="choice">
            <input
                type=(if value.is_some() { "radio" } else { "checkbox" })
                role=(value.is_none().then_some("switch"))
                id=(input.id.as_str())
                name=(name)
                value=(value)
                checked=(checked)
                autofocus=(autofocus)
                data-defaults=(defaults)
                aria-describedby=(input.described_by())
            >
            <label for=(input.id.as_str())>(label)</label>
            help(of: &input, (detail))
        </div>
    })
}

/// The question of a new client's kind, `chosen` checked. Each answer names the
/// switches its kind starts with on, so the client form script turns them on
/// when it is chosen.
#[component]
async fn kind_question(
    /// The kind checked.
    chosen: ClientKind,
) -> Result<impl View> {
    Ok(view! {
        <fieldset>
            <legend>"Does your app have a backend that can keep a secret?"</legend>
            for kind in [ClientKind::Confidential, ClientKind::Public] {
                let (answer, detail) = match kind {
                    ClientKind::Confidential => (
                        "Yes, a web app with a backend",
                        "Its server signs users in with a client secret.",
                    ),
                    ClientKind::Public => (
                        "No, it runs in the browser or on a device",
                        "Single-page, mobile and desktop apps, which cannot hide a secret.",
                    ),
                };
                choice(
                    name: "kind",
                    value: Some(kind.as_str()),
                    checked: kind == chosen,
                    defaults: Some(ClientDraft::new_client(kind).switches()),
                    label: view! { (answer) },
                    detail: view! { (detail) },
                )
            }
            <small>"Both sign in with PKCE (S256). The type cannot change later."</small>
        </fieldset>
    })
}

/// A switch for each standard scope, those of `chosen` checked, and why the
/// service refused them, as `scopes` says, if it did.
#[component]
async fn scope_choices<'a>(
    /// The scopes checked.
    chosen: ScopeChoice,
    /// The input of the scopes, with why they were refused if they were.
    scopes: &'a Described<'a>,
) -> Result<impl View> {
    Ok(view! {
        <fieldset aria-describedby=(scopes.refused().then(|| scopes.error()))>
            <legend>"Scopes"</legend>
            <div class="choices">
                for scope in Scope::ALL {
                    choice(
                        name: scope.as_str(),
                        checked: chosen.has(scope),
                        autofocus: scopes.refused() && scope == Scope::ALL[0],
                        label: view! { <code>(scope.as_str())</code> },
                        detail: view! { (scope_description(scope)) },
                    )
                }
            </div>
        </fieldset>
        refusal(of: scopes)
    })
}

/// A switch for each grant type, those of `chosen` checked, and why the service
/// refused them, as `grants` says, if it did.
#[component]
async fn grant_choices<'a>(
    /// The grant types checked.
    chosen: GrantChoice,
    /// The input of the grant types, with why they were refused if they were.
    grants: &'a Described<'a>,
) -> Result<impl View> {
    Ok(view! {
        <fieldset aria-describedby=(grants.refused().then(|| grants.error()))>
            <legend>"Grant types"</legend>
            <div class="choices">
                for grant in GrantType::ALL {
                    let (label, detail) = grant_description(grant);
                    choice(
                        name: grant.as_str(),
                        checked: chosen.has(grant),
                        autofocus: grants.refused() && grant == GrantType::ALL[0],
                        label: view! { (label) },
                        detail: view! { (detail) },
                    )
                }
            </div>
        </fieldset>
        refusal(of: grants)
    })
}

/// A client's settings form, posting `change` to `action` with a button labelled
/// `label`: its inputs show `draft`, and, with `kind`, it asks for a new client's
/// kind, `kind` chosen. Each input is named by its label alone, and described by
/// its help. The service's `error` shows beside the input of its setting, which
/// takes the focus, or above the inputs for a setting with no place for one (see
/// [`FieldError::placed`]). With `confirm`, the passkey script confirms with a
/// passkey first. The admin dashboard adds the inputs only operators fill in:
/// `head` after the name, and `child` after the rest.
#[component]
pub(crate) async fn client_form<'a>(
    /// Where the form posts.
    action: &'a str,
    /// The change the form makes.
    #[into]
    change: Fields,
    /// The values the inputs hold.
    draft: &'a ClientDraft,
    /// The kind chosen, when the form asks for a new client's kind.
    #[default]
    kind: Option<ClientKind>,
    /// What the button says.
    label: &'a str,
    /// Why the service refused a setting, if it did.
    #[default]
    error: Option<FieldError<'a>>,
    /// Whether a passkey confirms the change first.
    #[default]
    confirm: bool,
    #[into]
    #[default]
    /// The inputs only operators fill in, after the name.
    head: Child<'_>,
    /// The inputs only operators fill in, after the rest.
    #[default]
    child: Child<'_>,
) -> Result<impl View> {
    let unplaced = error.filter(|error| !error.placed());
    let name = Described::of(error, &ClientField::DisplayName);
    let redirects = Described::of(error, &ClientField::RedirectUris);
    let logouts = Described::of(error, &ClientField::PostLogoutUris);
    let scopes = Described::of(error, &ClientField::AllowedScopes);
    let grants = Described::of(error, &ClientField::GrantTypes);
    let backchannel = Described::of(error, &ClientField::BackchannelLogoutUri);
    Ok(view! {
        <div class="card">
            change_form(
                action: action,
                change: change,
                label: label,
                confirm: confirm,
                alert_notice(message: unplaced.map(|error| error.message))
                <label>
                    "App name"
                    <input
                        name="display_name"
                        value=(draft.display_name.as_str())
                        maxlength=(MAX_NAME_CHARS.to_string())
                        required=""
                        aria-invalid=(name.flag())
                        aria-describedby=(name.described_by())
                        autofocus=(name.refused())
                    >
                </label>
                help(of: &name, "Users see it when they sign in to your app.")
                refusal(of: &name)
                (head)
                if let Some(chosen) = kind {
                    kind_question(chosen: chosen)
                }
                <label>
                    "Redirect URIs"
                    <textarea
                        name="redirect_uris"
                        rows="3"
                        placeholder="https://app.example/auth/callback"
                        aria-invalid=(redirects.flag())
                        aria-describedby=(redirects.described_by())
                        autofocus=(redirects.refused())
                    >(draft.redirect_uris.text())</textarea>
                </label>
                help(
                    of: &redirects,
                    "One per line, matched exactly: where Origin Auth sends users back after signing in. HTTPS, or HTTP on a loopback IP for native apps. An app without the authorization code grant needs none."
                )
                refusal(of: &redirects)
                <label>
                    "Post-logout URIs"
                    <textarea
                        name="post_logout_uris"
                        rows="2"
                        placeholder="https://app.example/"
                        aria-invalid=(logouts.flag())
                        aria-describedby=(logouts.described_by())
                        autofocus=(logouts.refused())
                    >(draft.post_logout_uris.text())</textarea>
                </label>
                help(of: &logouts, "Optional, one per line: pages users may return to after signing out.")
                refusal(of: &logouts)
                scope_choices(chosen: draft.scopes, scopes: &scopes)
                grant_choices(chosen: draft.grants, grants: &grants)
                <div class="choices">
                    choice(
                        name: "require_pushed_authorization_requests",
                        checked: draft.require_pushed_authorization_requests.0,
                        label: view! { "Require pushed authorization requests" },
                        detail: view! {
                            "Your app must post its sign-in requests to " <code>"/oauth/par"</code> " (RFC 9126), so none of their parameters travel through the browser."
                        },
                    )
                </div>
                backchannel_input(uri: &draft.backchannel_logout_uri, of: &backchannel)
                (child)
            )
        </div>
    })
}

/// The input of a client's back-channel logout URI, showing `uri`, described as
/// `of` says.
#[component]
async fn backchannel_input<'a>(
    /// The URI the input shows.
    uri: &'a str,
    /// The input described.
    of: &'a Described<'a>,
) -> Result<impl View> {
    Ok(view! {
        <label>
            "Back-channel logout URI"
            <input
                name="backchannel_logout_uri"
                type="url"
                value=(uri)
                placeholder="https://app.example/auth/backchannel-logout"
                aria-invalid=(of.flag())
                aria-describedby=(of.described_by())
                autofocus=(of.refused())
            >
        </label>
        help(
            of: of,
            "Optional: where Origin Auth posts a logout token when a user's session ends, so your app can end its own. The Origin Auth SDKs serve it beside their login routes, e.g. at " <code>"/auth/backchannel-logout"</code> ". Origin Auth posts only to HTTPS URIs with public addresses."
        )
        refusal(of: of)
    })
}

/// A confidential client's `secrets`, oldest first, as of `now`: when each was
/// made, and when it stops or stopped working.
#[component]
pub(crate) async fn secrets_list<'a>(
    /// The client's secrets, oldest first.
    secrets: &'a [SecretRow],
    /// The time lifetimes are counted from.
    now: DateTime<Utc>,
) -> Result<impl View> {
    Ok(view! {
        <ul class="list">
            for secret in secrets {
                <li>
                    <div class="text">
                        <strong>"Created " time(at: secret.created_at, precision: Precision::Minute)</strong>
                        <small>
                            if let Some(at) = secret.expires_at {
                                (if at > now { "stops working " } else { "stopped working " })
                                time(at: at, precision: Precision::Minute)
                            } else {
                                "works until you rotate it"
                            }
                        </small>
                    </div>
                    match secret.expires_at {
                        Some(at) if at <= now => <span class="badge neutral">"Expired"</span>,
                        Some(_) => <span class="badge warning">"Expiring"</span>,
                        None => <span class="badge ok">"Current"</span>,
                    }
                </li>
            }
        </ul>
    })
}

/// A client `secret` Origin Auth just made, shown once, with a button copying it, on
/// `page`, whose address the passkey script, where the page loads it, puts back
/// so that reloading it gets the page without the secret; saving it leads on to
/// `next`.
#[component]
pub(crate) async fn secret_panel<'a>(
    /// The secret just made.
    secret: &'a Revealed,
    /// The page the panel is on.
    page: &'a str,
    /// Where saving the secret leads.
    next: &'a str,
) -> Result<impl View> {
    Ok(view! {
        <section class="card" aria-labelledby="secret-heading" data-replace-url=(page)>
            <header><h2 id="secret-heading" tabindex="-1" autofocus="">"Your client secret"</h2></header>
            <div class="body">
                <p class="notice warning" role="alert">
                    "This is the only time this secret is shown. Store it in your app's secret manager or environment now; Origin Auth keeps only a digest of it."
                </p>
                copyable(id: "client-secret", label: "client secret", value: secret.expose())
                <div class="row"><a class="button" href=(next)>"I saved it"</a></div>
            </div>
        </section>
    })
}

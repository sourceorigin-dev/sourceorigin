//! The account's data: downloading it, its part in usage statistics, and
//! deleting the account.

use super::Part;
use crate::{
    form::{Checkbox, Precision, Query, change_form, open, time},
    layout::page_head,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use topcoat::{
    Result,
    view::{View, component, view},
};

/// A change the data page posts, named by its `action` field. Downloading and
/// deleting need a recent passkey confirmation. A field the user types is left
/// empty here, and the form renders its input instead.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DataChange {
    /// Downloads every record of the account.
    Export,
    /// Turns the account's part in usage statistics on, or off.
    Statistics {
        /// Whether it takes part from now on.
        #[serde(default, skip_serializing_if = "Checkbox::is_off")]
        included: Checkbox,
    },
    /// Schedules the account's deletion.
    Delete {
        /// The account's handle, typed to confirm.
        #[serde(skip_serializing_if = "String::is_empty")]
        handle: String,
    },
}

/// Whether the data page's deletion row is open, as its query names it.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct DataState {
    /// The deletion awaits its confirmation.
    #[serde(rename = "delete", default)]
    pub deleting: bool,
}

impl Query for DataState {}

/// The data page: downloading the account's records, whether it takes part in
/// usage statistics (`statistics`), and deleting the account, whose `handle`
/// confirms it, after `grace_days` days. Its changes post to `page`, which
/// `state` opens; `confirmed` says whether the session needs no passkey
/// confirmation for them.
#[component]
pub async fn account_data<'a>(
    /// Where the page is, which its changes post to.
    page: &'a str,
    /// The handle the owner types to confirm deleting the account.
    handle: &'a str,
    /// Whether the account takes part in usage statistics.
    statistics: bool,
    /// Days the owner has to cancel deletion.
    grace_days: i64,
    /// What the page is showing of the download and the deletion.
    state: DataState,
    /// Whether the deletion form is confirmed.
    confirmed: bool,
) -> Result<impl View> {
    Ok(view! {
        page_head(title: "Your data", lead: "Everything SourceOrigin keeps about you, and leaving.".to_owned())
        <section class="card" aria-labelledby=(Part::Export.id())>
            <header>
                <h2 id=(Part::Export.id())>"Download your data"</h2>
                change_form(
                    action: page,
                    change: DataChange::Export,
                    label: "Download your data",
                    class: "secondary small",
                    confirm: !confirmed,
                    download: true,
                )
            </header>
            <p class="help">
                "Everything SourceOrigin keeps about your account, in one JSON file: your profile and its picture, handles, email addresses, passkeys and linked accounts, devices, connected apps, the apps you registered, account recoveries, your inbox and every event recorded about your account. What you removed is in it until it is deleted. It holds no passkey keys or secrets."
            </p>
        </section>
        <section class="card" aria-labelledby=(Part::Statistics.id())>
            <header>
                <h2 id=(Part::Statistics.id())>"Usage statistics"</h2>
                change_form(
                    action: page,
                    change: DataChange::Statistics { included: Checkbox(!statistics) },
                    label: "Include my activity in usage statistics",
                    on: Some(statistics),
                )
            </header>
            <p class="help">
                "SourceOrigin counts how it is used, such as which kinds of passkeys people sign in with, to see what works and what fails. The counts never say who you are. Turn this off to leave yours out. A browser that asks sites not to track it, with Global Privacy Control, is always left out."
            </p>
        </section>
        <section class="card" aria-labelledby=(Part::Delete.id())>
            <header>
                <h2 id=(Part::Delete.id())>"Delete your account"</h2>
                if !state.deleting {
                    open(
                        page: page,
                        opens: DataState { deleting: true },
                        label: "Delete account",
                        danger: true,
                    )
                }
            </header>
            <p class="help">
                (format!(
                    "Your account stops working at once and every device is signed out. It is deleted for good after {grace_days} days, with the apps you registered; until then, signing in with your passkey restores it. Apps you signed in to keep their own records of you."
                ))
            </p>
            if state.deleting {
                <div class="open-row">
                    <strong tabindex="-1" autofocus="">"Delete your account?"</strong>
                    change_form(
                        action: page,
                        change: DataChange::Delete { handle: String::new() },
                        label: "Delete my account",
                        class: "danger",
                        confirm: !confirmed,
                        cancel: page.to_owned(),
                        <label>
                            (format!("Type your handle, {handle}, to confirm"))
                            <input
                                name="handle"
                                autocomplete="off"
                                autocapitalize="none"
                                spellcheck="false"
                                required=""
                            >
                        </label>
                    )
                </div>
            }
        </section>
    })
}

/// How the history says the account's part in usage statistics changed to
/// `included`, e.g. "Usage statistics turned off".
pub(crate) const fn statistics_change(included: bool) -> &'static str {
    if included {
        "Usage statistics turned on"
    } else {
        "Usage statistics turned off"
    }
}

/// The account just scheduled for deletion, which is purged at `purge_after`
/// unless its owner signs in at `sign_in` with their passkey before then.
#[component]
pub async fn account_deleted<'a>(
    /// When the account is purged.
    purge_after: DateTime<Utc>,
    /// Where the owner signs in to stop it.
    sign_in: &'a str,
) -> Result<impl View> {
    Ok(view! {
        <h1>"Your account is scheduled for deletion"</h1>
        <p class="lead">
            "Every device was signed out. Your account will be deleted for good on "
            time(at: purge_after, precision: Precision::Day)
            ". Changed your mind? Sign in with your passkey before then to restore it."
        </p>
        <a class="button secondary" href=(sign_in)>"Restore my account"</a>
    })
}

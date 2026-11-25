//! The parts of a page within its frame: its opening and messages, its
//! navigations, the pager below a list, a private address, and an
//! announcement's text.

use crate::{
    Alert, Messages,
    icons::{Icon, icon},
};
use sourceorigin_common::admin::{Announcement, Paging};
use topcoat::{
    Result,
    view::{Child, View, component, view},
};

/// Something a navigation links, by its label.
pub(crate) trait Label: Copy + PartialEq + Send + Sync + 'static {
    /// The `aria-current` of the current one's link: `page` when the link opens
    /// the page shown, as a tab's does.
    const CURRENT: &'static str = "page";

    /// The link's text.
    fn label(self) -> &'static str;

    /// Whether its page needs the dashboard's passkey confirmation, so the
    /// dashboard's script confirms before following its link.
    fn confirms_first(self) -> bool {
        false
    }
}

/// Where a list's page stands, and the links to the pages before and after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pager {
    /// Where the page stands.
    pub paging: Paging,
    /// The previous page, if there is one.
    pub previous: Option<String>,
    /// The next page, if there is one.
    pub next: Option<String>,
}

impl Pager {
    /// The pager of `paging`, linking each page `link` names.
    ///
    /// # Errors
    ///
    /// Fails as `link` does.
    pub fn of<E>(
        paging: Paging,
        mut link: impl FnMut(u32) -> Result<String, E>,
    ) -> Result<Self, E> {
        Ok(Self {
            previous: paging.previous().map(&mut link).transpose()?,
            next: paging.next().map(&mut link).transpose()?,
            paging,
        })
    }
}

/// The pager below a list of `what`: which items the page shows of how many, and
/// the previous and next pages. A list that fits one page shows only its count.
#[component]
pub(crate) async fn pager<'a>(
    /// The page of items shown, among how many.
    pages: &'a Pager,
    /// What the items are called.
    what: &'a str,
) -> Result<impl View> {
    let paging = pages.paging;
    let shown = match paging.range() {
        Some((first, last)) => format!("{first}–{last} of {} {what}", paging.total),
        None => format!("No {what}"),
    };
    Ok(view! {
        <nav class="pager" aria-label="Pages">
            <span class="muted">(shown.as_str())</span>
            if paging.pages > 1 {
                <span class="row">
                    match &pages.previous {
                        Some(href) => <a class="button secondary small" href=(href.as_str()) rel="prev">"Previous"</a>,
                        None => <span class="button secondary small" aria-disabled="true">"Previous"</span>,
                    }
                    <span class="mono muted">(format!("page {} of {}", paging.page, paging.pages))</span>
                    match &pages.next {
                        Some(href) => <a class="button secondary small" href=(href.as_str()) rel="next">"Next"</a>,
                        None => <span class="button secondary small" aria-disabled="true">"Next"</span>,
                    }
                </span>
            }
        </nav>
    })
}

/// A page's opening: its `title`, a `lead` paragraph, if any, and what acts on
/// the whole page as `actions`, beside them.
#[component]
pub async fn page_head(
    /// What the page is titled.
    #[into]
    title: String,
    #[into]
    #[default]
    /// A paragraph under the title, if any.
    lead: Option<String>,
    #[into]
    #[default]
    /// What acts on the whole page.
    actions: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <div class="page-head">
            <div>
                <h1>(title)</h1>
                if let Some(lead) = &lead {
                    <p class="lead">(lead.as_str())</p>
                }
            </div>
            <div class="row">(actions)</div>
        </div>
    })
}

/// A text, `body`, as paragraphs, keeping its line breaks: an announcement's as
/// its administrators wrote it, or any notification's.
#[component]
pub(crate) async fn announcement_text<'a>(
    /// The text.
    body: &'a str,
) -> Result<impl View> {
    let paragraphs = Announcement::paragraphs(body);
    Ok(view! {
        for lines in &paragraphs {
            <p>
                for (index, line) in lines.iter().enumerate() {
                    if index > 0 {
                        <br>
                    }
                    (*line)
                }
            </p>
        }
    })
}

/// What a page says: the `alert` of a change that failed, and the `notice` of
/// the one just made.
#[component]
pub(crate) async fn page_messages<'a>(
    /// The failure and the notice to show.
    messages: Messages<'a>,
) -> Result<impl View> {
    Ok(view! {
        if let Some(Alert::Page(alert)) = messages.alert {
            alert_notice(message: alert)
        }
        if let Some(notice) = messages.notice {
            <p class="notice" role="status">
                icon(kind: Icon::Check)
                <span class="text">(notice.message())</span>
                <label class="dismiss" title="Dismiss">
                    <input type="checkbox" aria-label="Dismiss">
                    icon(kind: Icon::Close)
                </label>
            </p>
        }
    })
}

/// Why a change failed, as `message` says it, or nothing without one. A
/// `passkey` notice is instead the placeholder a page's passkey script says a
/// failed ceremony in: hidden until then, when the script writes why into its
/// `.text` and shows it.
#[component]
pub(crate) async fn alert_notice<'a>(
    #[into]
    #[default]
    /// Why the change failed, if it did.
    message: Option<&'a str>,
    /// Whether the notice is the passkey script's placeholder.
    #[default]
    passkey: bool,
) -> Result<impl View> {
    Ok(view! {
        if message.is_some() || passkey {
            <p class="notice error" role="alert" data-passkey-error=(passkey) hidden=(passkey)>
                icon(kind: Icon::Warning)
                <span class="text">(message.unwrap_or_default())</span>
            </p>
        }
    })
}

/// A dialog titled `title`, its element `id`, holding `child`. A button naming
/// `id` as its `popovertarget` opens it, and the browser closes it on Escape or a
/// click outside, with no script. Once `shown`, as when a change it holds was
/// refused, it is open as the page loads, saying the refusal's `alert` above its
/// form, over a scrim, and closing it, by its Close or a click outside, goes
/// `back`. A dialog that asks no input says to `focus_close`, so the focus moves
/// into it as it opens; one that does autofocuses its input.
#[component]
pub(crate) async fn dialog<'a>(
    /// The dialog's element ID, which a button names to open it.
    id: &'a str,
    /// What the dialog is titled.
    title: &'a str,
    /// Whether the dialog is open as the page loads.
    shown: bool,
    /// Where closing it goes.
    back: &'a str,
    /// Why the last attempt failed, if it did.
    #[default]
    alert: Option<&'a str>,
    /// Whether the focus moves to the close button as it opens.
    #[default]
    focus_close: bool,
    /// What the dialog holds.
    child: Child<'_>,
) -> Result<impl View> {
    let heading = format!("{id}-title");
    Ok(view! {
        if shown {
            <a class="dialog-scrim" href=(back) tabindex="-1" aria-hidden="true"></a>
        }
        <div
            class=(if shown { "dialog shown" } else { "dialog" })
            id=(id)
            popover=((!shown).then_some(""))
            role="dialog"
            aria-labelledby=(heading.as_str())
        >
            <header>
                <h2 id=(heading.as_str())>(title)</h2>
                if shown {
                    <a class="close" href=(back) aria-label="Close" autofocus=(focus_close)>icon(kind: Icon::Close)</a>
                } else {
                    <button type="button" class="close" popovertarget=(id) popovertargetaction="hide" aria-label="Close" autofocus=(focus_close)>
                        icon(kind: Icon::Close)
                    </button>
                }
            </header>
            alert_notice(message: alert.filter(|_| shown))
            (child)
        </div>
    })
}

/// An email `address`, blurred until clicked so a shared screen does not show it;
/// clicking again blurs it. A checkbox, visually hidden, holds whether it shows,
/// so it needs no script.
#[component]
pub(crate) async fn private_address<'a>(
    /// The email address.
    address: &'a str,
) -> Result<impl View> {
    Ok(view! {
        <label class="private-address">
            <input type="checkbox" aria-label="Show address">
            <span>(address)</span>
        </label>
    })
}

/// The navigation named `label`, of `class`, linking each of `links` to its URL,
/// `current` among them, whose link is marked [`Label::CURRENT`], and marked
/// `data-confirm-first` when [`Label::confirms_first`].
#[component]
pub(crate) async fn navigation<'a, T: Label>(
    /// What the navigation is named for assistive technology.
    label: &'a str,
    /// The navigation's CSS class.
    class: &'a str,
    /// The links, each with its URL.
    links: &'a [(T, &'a str)],
    /// The link marked as the current page.
    current: T,
) -> Result<impl View> {
    Ok(view! {
        <nav class=(class) aria-label=(label)>
            for (link, href) in links {
                <a href=(*href) aria-current=((*link == current).then_some(T::CURRENT)) data-confirm-first=(link.confirms_first())>(link.label())</a>
            }
        </nav>
    })
}

//! The outbound delivery queue.

use super::{Action, Badge, Change, badge, confirm, data_table, moment};
use crate::layout::page_head;
use sourceorigin_common::admin::{DeliveryStatus, DeliverySummary};
use topcoat::{
    Result,
    view::{View, component, view},
};

/// A delivery, and the changes an administrator may make to it now.
#[derive(Clone, Debug)]
pub struct DeliveryRow<'a> {
    /// The delivery.
    pub delivery: &'a DeliverySummary,
    /// Attempting it again, while it may be.
    pub retry: Option<Action>,
    /// Cancelling it, while it may be.
    pub cancel: Option<Action>,
}

/// A delivery's status, as a badge.
const fn status_badge(status: DeliveryStatus) -> Badge {
    match status {
        DeliveryStatus::Pending => Badge::new("warning", "Pending"),
        DeliveryStatus::Succeeded => Badge::new("ok", "Delivered"),
        DeliveryStatus::Failed => Badge::new("danger", "Failed"),
        DeliveryStatus::Cancelled => Badge::new("neutral", "Cancelled"),
    }
}

/// The newest deliveries, at most `limit`, `rows`, with the `status` and on the
/// `channel` chosen, if any, and the form choosing others, which sends them to
/// `action` by `GET`.
#[component]
pub async fn delivery_list<'a>(
    /// Where the filter form posts.
    action: &'a str,
    #[into]
    #[default]
    /// The status chosen, if any.
    status: Option<DeliveryStatus>,
    #[into]
    #[default]
    /// The channel chosen, if any.
    channel: Option<&'a str>,
    /// The deliveries, newest first.
    rows: &'a [DeliveryRow<'a>],
    /// The most deliveries shown.
    limit: u32,
) -> Result<impl View> {
    let full = u32::try_from(rows.len()).is_ok_and(|shown| shown >= limit);
    Ok(view! {
        page_head(
            title: "Deliveries",
            lead: "Emails and back-channel logouts, newest first. A failed delivery is not attempted again until it is retried.".to_owned(),
        )
        <form method="get" action=(action) class="row search">
            <select name="status" aria-label="Status">
                <option value="" selected=(status.is_none())>"Every status"</option>
                for each in DeliveryStatus::ALL {
                    <option value=(each.as_str()) selected=(status == Some(each))>(status_badge(each).label)</option>
                }
            </select>
            <input name="channel" value=(channel) placeholder="Channel, e.g. email" aria-label="Channel">
            <button class="secondary">"Filter"</button>
        </form>
        <section class="card">
            data_table(
                headings: &["destination", "status", "attempts", "due or finished", "last error", ""],
                empty: "No delivery.",
                filled: !rows.is_empty(),
                for row in rows {
                    let delivery = row.delivery;
                    let subject = delivery.id.to_string();
                    <tr>
                        <td>
                            <code>(delivery.channel.as_str())</code> " " (delivery.destination.as_str())
                        </td>
                        <td>badge(shown: status_badge(delivery.status))</td>
                        <td>(delivery.attempts.to_string())</td>
                        <td>moment(at: delivery.finished_at.unwrap_or(delivery.next_attempt_at))</td>
                        <td><small>(delivery.last_error.as_deref().unwrap_or("—"))</small></td>
                        <td class="right">
                            <span class="row">
                                if let Some(retry) = &row.retry {
                                    confirm(action: retry, change: Change::RetryDelivery, subject: subject.as_str())
                                }
                                if let Some(cancel) = &row.cancel {
                                    confirm(action: cancel, change: Change::CancelDelivery, subject: subject.as_str())
                                }
                            </span>
                        </td>
                    </tr>
                }
            )
        </section>
        if full {
            <p class="help">(format!("Only the newest {limit} are shown."))</p>
        }
    })
}

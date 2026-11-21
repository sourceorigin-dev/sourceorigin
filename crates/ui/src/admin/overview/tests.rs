//! The overview page's words.

use super::{AdminOverview, admin_overview};
use topcoat::{
    context::Cx,
    view::{ViewExt, view},
};

/// One account, or one administrator, reads as a singular on the page.
#[tokio::test]
async fn the_lead_counts_accounts_in_the_right_number() {
    for (accounts, administrators, counts) in [
        (1, 1, "1 account, 1 of them an administrator."),
        (1, 0, "1 account, 0 of them administrators."),
        (2, 1, "2 accounts, 1 of them an administrator."),
        (2, 3, "2 accounts, 3 of them administrators."),
    ] {
        let overview = AdminOverview {
            problems: &[],
            measures: &[],
            accounts,
            administrators,
            events: &[],
            newest: &[],
            all_events: "",
            all_users: "",
        };
        let cx = Cx::default();
        let page = view! { cx => admin_overview(overview: overview) }
            .single()
            .await
            .expect("the overview")
            .render(&cx);
        assert!(page.contains(counts), "{page}");
    }
}

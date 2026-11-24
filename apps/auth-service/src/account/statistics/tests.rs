//! Leaving usage statistics, and coming back.

use super::*;
use crate::{
    security::RequestContext,
    tests::{Browser, Replicas},
};

/// An account takes part by default. Its owner leaves and comes back, each
/// change recorded once, and while it is out, or its browser asks for Global
/// Privacy Control, no product row may be recorded about it.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn the_owner_leaves_usage_statistics_and_comes_back() {
    let database = Replicas::database().await;
    let service = AccountService::new(database.clone());
    let alice = Browser::new_account(&database).await;
    let owner = alice.owner(&database).await;
    let consented = async |context: RequestContext| {
        let mut tx = database.begin().await.expect("transaction");
        let consent = context
            .consent(&mut tx, Some(alice.user_id))
            .await
            .expect("consent");
        tx.commit().await.expect("commit");
        consent.is_some()
    };
    assert!(
        service
            .usage_statistics(&Viewer::from(&owner))
            .await
            .expect("read")
    );
    assert!(consented(RequestContext::TEST).await, "included by default");
    let gpc = RequestContext {
        gpc: true,
        ..RequestContext::TEST
    };
    assert!(
        !consented(gpc).await,
        "Global Privacy Control leaves it out"
    );

    for included in [false, false] {
        service
            .set_usage_statistics(&owner, included)
            .await
            .expect("left");
    }
    assert!(
        !service
            .usage_statistics(&Viewer::from(&owner))
            .await
            .expect("read")
    );
    assert!(!consented(RequestContext::TEST).await, "left out");
    service
        .set_usage_statistics(&owner, true)
        .await
        .expect("back");
    assert!(consented(RequestContext::TEST).await, "back in");

    let changes: Vec<bool> = database
        .events()
        .history_for_user(alice.user_id, 20)
        .await
        .expect("history")
        .into_iter()
        .filter_map(|stored| match stored.event.known() {
            Some(DomainEvent::UsageStatisticsChanged { included }) => Some(*included),
            _ => None,
        })
        .collect();
    assert_eq!(
        changes,
        [true, false],
        "newest first; a repeat changes nothing"
    );
}

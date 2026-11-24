//! Whether the account takes part in usage statistics: whether auth records
//! product analytics about what its owner does, such as which kind of passkey
//! they sign in with. See `docs/dev/architecture/analytics.md#consent`.
//!
//! Only the owner changes it, as their own privacy choice, and only they see it.

use super::{AccountError, AccountService, Owner, Viewer};
use sourceorigin_common::events::DomainEvent;

impl AccountService {
    /// Whether the account takes part in usage statistics.
    pub(crate) async fn usage_statistics(&self, viewer: &Viewer) -> Result<bool, AccountError> {
        viewer.authorize(&self.database).await?;
        Ok(self
            .database
            .users()
            .analytics_included(viewer.user())
            .await?)
    }

    /// Includes the owner's account in usage statistics, or leaves them, under
    /// the account lock, and records the change if it made one. Their session
    /// is enough: it only narrows what auth records.
    pub(crate) async fn set_usage_statistics(
        &self,
        owner: &Owner,
        included: bool,
    ) -> Result<(), AccountError> {
        let mut tx = self.database.begin().await?;
        owner.lock(&mut tx).await?;
        if tx
            .users()
            .set_analytics_included(owner.user(), included)
            .await?
        {
            let event = DomainEvent::UsageStatisticsChanged { included };
            tx.record(&owner.context(), Some(owner.user()), event)
                .await?;
        }
        Ok(tx.commit().await?)
    }
}

#[cfg(test)]
mod tests;

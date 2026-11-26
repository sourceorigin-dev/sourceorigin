//! Looking accounts up in auth's public directory, to name the accounts whose
//! IDs an application's records keep.

use crate::{
    Issuer, UserId,
    http::{Authorization, NetworkError},
};
use sourceorigin_common::protocol::{
    directory::{ACCOUNTS_PATH, AccountLookup, MAX_LOOKUP_IDS, PublicAccount},
    sdk::{ErrorCategory, SdkError},
};
use thiserror::Error;

/// The directory could not be read.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LookupError {
    /// The issuer's identifier does not make a lookup URL.
    #[error("issuer {0:?} has no account lookup URL")]
    InvalidIssuer(String, #[source] url::ParseError),
    /// The directory could not be reached, or refused the lookup.
    #[error(transparent)]
    Network(#[from] NetworkError),
    /// The answer is not a lookup's.
    #[error("the account lookup's answer is invalid")]
    InvalidDocument(#[source] serde_json::Error),
}

impl LookupError {
    /// The shared SDK error code.
    #[must_use]
    pub const fn code(&self) -> SdkError {
        SdkError::category(ErrorCategory::Network)
    }
}

impl Issuer {
    /// What is public about the accounts `ids`: the current handle of each, to
    /// show beside the ID a record keeps, which a rename never changes. An
    /// account that is disabled, scheduled for deletion or deleted is left out,
    /// as an ID no account has is, so show such an ID as a former account. IDs
    /// are asked for [`MAX_LOOKUP_IDS`] at a time.
    ///
    /// ```no_run
    /// # async fn run(issuer: sourceorigin_auth::Issuer, author: sourceorigin_auth::UserId) -> Result<(), sourceorigin_auth::LookupError> {
    /// let accounts = issuer.accounts(&[author]).await?;
    /// match accounts.first() {
    ///     Some(account) => println!("by @{}", account.handle),
    ///     None => println!("by a former account"),
    /// }
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// See [`LookupError`].
    pub async fn accounts(&self, ids: &[UserId]) -> Result<Vec<PublicAccount>, LookupError> {
        let lookup = Self::under(self.issuer(), ACCOUNTS_PATH)
            .map_err(|source| LookupError::InvalidIssuer(self.issuer().to_owned(), source))?;
        let mut accounts = Vec::new();
        for batch in ids.chunks(MAX_LOOKUP_IDS) {
            let mut url = lookup.clone();
            url.query_pairs_mut()
                .extend_pairs(batch.iter().map(|id| ("id", id.to_string())));
            let reply = self.http().get(&url, Authorization::None).await?.ok(&url)?;
            let found: AccountLookup =
                serde_json::from_slice(&reply.body).map_err(LookupError::InvalidDocument)?;
            accounts.extend(found.accounts);
        }
        Ok(accounts)
    }
}

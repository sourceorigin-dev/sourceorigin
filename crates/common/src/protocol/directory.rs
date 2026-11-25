//! The public directory: what anyone may read about an account from its ID.
//! Records keep an account's ID, which never changes, and ask here for the name
//! to show beside it.
//!
//! ```
//! use sourceorigin_common::protocol::directory::AccountLookup;
//!
//! let found: AccountLookup = serde_json::from_str(
//!     r#"{"accounts": [{"id": "0199a000-0000-7000-8000-000000000001", "handle": "ada"}]}"#,
//! )?;
//! assert_eq!(found.accounts[0].handle.as_str(), "ada");
//! # Ok::<(), serde_json::Error>(())
//! ```

use crate::UserId;
use serde::{Deserialize, Serialize};

super::sdk_constants! {
    /// Path of the account lookup, under the issuer: `GET` with one `id` query
    /// parameter per account.
    pub const ACCOUNTS_PATH: &str = "/accounts";
    /// The most account IDs one lookup may name.
    pub const MAX_LOOKUP_IDS: usize = 100;
}

/// What is public about one account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PublicAccount {
    /// The account's ID, which never changes: the `sub` of its tokens.
    pub id: UserId,
    /// Its handle now. A rename changes it.
    pub handle: String,
}

/// The answer to a lookup: the accounts, among those asked for, that can be
/// named. An account that is disabled, scheduled for deletion or deleted, and an
/// ID no account has, are left out, and nothing tells them apart.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AccountLookup {
    /// The accounts found, by ID.
    pub accounts: Vec<PublicAccount>,
}

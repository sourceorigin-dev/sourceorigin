//! The public directory: what anyone may read about an account from its ID.
//! Services that record account IDs, which never change, ask here for the
//! handle to show beside each. Handles are public
//! (`docs/dev/security/handles.md`), and the lookup follows a rename.
//!
//! An account that is disabled, scheduled for deletion or deleted is left out,
//! as an ID no account has is: the answer tells none of them apart.

use crate::{routes::directory::ACCOUNTS, security::ApiError};
use axum::{
    Json, Router,
    extract::{RawQuery, State},
    routing::get,
};
use sourceorigin_common::{
    UserId,
    protocol::directory::{AccountLookup, MAX_LOOKUP_IDS},
};
use sourceorigin_repository::Database;
use std::collections::HashSet;

/// The public lookup route.
#[derive(Clone)]
pub(crate) struct DirectoryApplication {
    /// Where accounts are read.
    database: Database,
}

impl DirectoryApplication {
    /// Builds the route over `database`.
    pub(crate) const fn new(database: Database) -> Self {
        Self { database }
    }

    /// Returns the lookup route with this application as state.
    pub(crate) fn routes(self) -> Router {
        Router::new()
            .route(ACCOUNTS, get(Self::accounts))
            .with_state(self)
    }

    /// The account IDs a lookup's `query` names, one per `id` parameter, each
    /// once. Other parameters are ignored.
    ///
    /// # Errors
    /// Refuses a query that names none, more than [`MAX_LOOKUP_IDS`], or an `id`
    /// that is no account ID, at `400`.
    fn ids(query: Option<&str>) -> Result<Vec<UserId>, ApiError> {
        let named = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
            .filter(|(name, _)| name == "id")
            .map(|(_, id)| {
                id.parse::<UserId>().map_err(|error| {
                    ApiError::bad_request(format!("id {id:?} is not an account ID: {error}"))
                })
            })
            .collect::<Result<HashSet<_>, _>>()?;
        match named.len() {
            0 => Err(ApiError::bad_request(
                "name at least one account: ?id=<account ID>",
            )),
            count if count > MAX_LOOKUP_IDS => Err(ApiError::bad_request(format!(
                "a lookup names at most {MAX_LOOKUP_IDS} accounts"
            ))),
            _ => Ok(named.into_iter().collect()),
        }
    }

    /// `GET /accounts?id=…&id=…`: the handle of each active account among those
    /// named, by ID.
    async fn accounts(
        State(app): State<Self>,
        RawQuery(query): RawQuery,
    ) -> Result<Json<AccountLookup>, ApiError> {
        let ids = Self::ids(query.as_deref())?;
        let accounts = app.database.users().public(&ids).await?;
        Ok(Json(AccountLookup { accounts }))
    }
}

/// The lookup against a real database.
#[cfg(test)]
mod tests;

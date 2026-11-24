//! Account records and locks that serialize security-sensitive mutations.

use super::{RepositoryExecutor, Unsigned, UserRepository};
use crate::DatabaseError;
use chrono::{DateTime, Utc};
use sourceorigin_common::{AccountStatus, Handle, PictureId, UserId, admin::User};

/// A user account row, with its current handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredUser {
    /// Immutable account identifier.
    pub id: UserId,
    /// Public identifier and login name.
    pub handle: Handle,
    /// When the handle last changed, or `None` while it is the one chosen at
    /// signup.
    pub handle_changed_at: Option<DateTime<Utc>>,
    /// Name shown to the user and to relying parties.
    pub display_name: String,
    /// Starts at 1; bumped when profile claims change.
    pub profile_version: u64,
    /// The current profile picture, while the account is active, as auth serves
    /// it only then.
    pub picture: Option<PictureId>,
    /// Whether the account can authenticate.
    pub status: AccountStatus,
    /// When the account was created.
    pub created_at: DateTime<Utc>,
    /// When the profile last changed.
    pub updated_at: DateTime<Utc>,
}

/// A [`query_record!`] over accounts `u`, with their current handle `h` and, while
/// active, picture `p`: it returns [`StoredUser`]s, or with `admin` first the
/// [`User`]s administrators see. `$rest` follows the joins, e.g.
/// `users!("WHERE u.id = $1", id)`.
macro_rules! users {
    (admin $($query:tt)*) => {
        users!(
            @ User,
            r#"u.id AS "id: UserId", h.handle AS "handle: Handle", u.display_name,
            u.status AS "status: AccountStatus", p.id AS "picture?: PictureId", u.created_at"#,
            $($query)*
        )
    };
    (@ $record:ident, $columns:literal, $rest:literal $(, $arg:expr)* $(,)?) => {
        query_record!(
            $record,
            $columns,
            "SELECT",
            "FROM users u
            JOIN handles h ON h.user_id = u.id AND h.released_at IS NULL
            LEFT JOIN profile_pictures p ON p.user_id = u.id AND u.status = 'active' "
                + $rest
            $(, $arg)*
        )
    };
    ($($query:tt)*) => {
        users!(
            @ StoredUser,
            r#"u.id AS "id: UserId", h.handle AS "handle: Handle", u.handle_changed_at,
            u.display_name, u.profile_version AS "profile_version: Unsigned",
            u.status AS "status: AccountStatus", u.created_at, u.updated_at,
            p.id AS "picture?: PictureId""#,
            $($query)*
        )
    };
}

impl From<StoredUser> for User {
    fn from(user: StoredUser) -> Self {
        Self {
            id: user.id,
            handle: user.handle,
            display_name: user.display_name,
            status: user.status,
            picture: user.picture,
            created_at: user.created_at,
        }
    }
}

/// A new active account.
#[derive(Clone, Debug)]
pub struct InsertUser {
    /// Identifier for the new account.
    pub id: UserId,
    /// Handle, also the login name. Reserved names are the caller's concern.
    pub handle: Handle,
    /// Name shown to the user and to relying parties.
    pub display_name: String,
}

impl UserRepository<'_> {
    /// Creates an active account holding `input.handle`, in one statement.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::Conflict`] if the id is taken or another account
    /// holds the handle or one that looks like it, or another [`DatabaseError`] if
    /// the query fails.
    pub async fn insert(&mut self, input: &InsertUser) -> Result<StoredUser, DatabaseError> {
        run!(
            self,
            fetch_one,
            sqlx::query_as!(
                StoredUser,
                r#"WITH u AS (
                    INSERT INTO users (id, display_name) VALUES ($1, $3) RETURNING *
                ), h AS (
                    INSERT INTO handles (handle, user_id) VALUES ($2, $1) RETURNING handle
                )
                SELECT
                    u.id AS "id: UserId", h.handle AS "handle: Handle", u.handle_changed_at,
                    u.display_name, u.profile_version AS "profile_version: Unsigned",
                    u.status AS "status: AccountStatus", u.created_at, u.updated_at,
                    NULL::uuid AS "picture?: PictureId"
                FROM u, h"#,
                input.id.as_uuid(),
                input.handle.as_str(),
                input.display_name,
            )
        )
    }

    /// Finds an account in any status.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or the row is invalid.
    pub async fn find_by_id(&mut self, id: UserId) -> Result<Option<StoredUser>, DatabaseError> {
        run!(
            self,
            fetch_optional,
            users!("WHERE u.id = $1", id.as_uuid())
        )
    }

    /// Finds an active account and locks it until the transaction ends.
    ///
    /// Account mutations serialize on this lock. `FOR NO KEY UPDATE` still
    /// lets inserts that reference the account check their foreign keys
    /// concurrently. Only the account row is locked, not its handle.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or the row is invalid.
    pub async fn find_active_by_id_for_update(
        &mut self,
        id: UserId,
    ) -> Result<Option<StoredUser>, DatabaseError> {
        self.lock(id, true).await
    }

    /// Locks the account `id`, only if active when `active`, then reads it. A lock
    /// that waited sees the account's latest row, but a join in the same statement
    /// would keep the handle its snapshot saw, perhaps from before a rename that
    /// committed meanwhile. The read is a separate statement, with a fresh snapshot.
    async fn lock(
        &mut self,
        id: UserId,
        active: bool,
    ) -> Result<Option<StoredUser>, DatabaseError> {
        let locked = run!(
            self,
            fetch_optional,
            sqlx::query_scalar!(
                "SELECT id FROM users
                WHERE id = $1 AND (NOT $2 OR status = 'active')
                FOR NO KEY UPDATE",
                id.as_uuid(),
                active,
            )
        )?;
        match locked {
            Some(_) => self.find_by_id(id).await,
            None => Ok(None),
        }
    }

    /// Finds an active account by its current handle, for tests that look up
    /// the account a flow created.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or the row is invalid.
    #[cfg(any(test, feature = "test-util"))]
    pub async fn find_active_by_handle(
        &mut self,
        handle: &Handle,
    ) -> Result<Option<StoredUser>, DatabaseError> {
        run!(
            self,
            fetch_optional,
            users!(
                "WHERE h.handle = $1
                AND u.status = 'active'",
                handle.as_str(),
            )
        )
    }

    /// Finds an account in any status and locks it until the transaction ends, like
    /// [`Self::find_active_by_id_for_update`].
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or the row is invalid.
    pub async fn find_by_id_for_update(
        &mut self,
        id: UserId,
    ) -> Result<Option<StoredUser>, DatabaseError> {
        self.lock(id, false).await
    }

    /// Finds an account in any status by its current handle.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or the row is invalid.
    pub async fn find_by_handle(
        &mut self,
        handle: &Handle,
    ) -> Result<Option<StoredUser>, DatabaseError> {
        run!(
            self,
            fetch_optional,
            users!("WHERE h.handle = $1", handle.as_str())
        )
    }

    /// Records that the account's handle changed now, and bumps its profile version
    /// so relying parties refresh `preferred_username`.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub(crate) async fn handle_changed(&mut self, id: UserId) -> Result<(), DatabaseError> {
        run!(
            self,
            execute,
            sqlx::query!(
                "UPDATE users
                SET handle_changed_at = now(), profile_version = profile_version + 1,
                    updated_at = now()
                WHERE id = $1",
                id.as_uuid(),
            )
        )?;
        Ok(())
    }

    /// Gives the account `id` the display name `name`, and bumps its profile version
    /// so relying parties refresh `name`. Returns the previous name, or `None` if the
    /// account already had this one, or does not exist: nothing changes then.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn rename(
        &mut self,
        id: UserId,
        name: &str,
    ) -> Result<Option<String>, DatabaseError> {
        Ok(run!(
            self,
            fetch_optional,
            sqlx::query_scalar!(
                "UPDATE users u
                SET display_name = $2, profile_version = u.profile_version + 1,
                    updated_at = now()
                FROM users old
                WHERE u.id = $1 AND old.id = u.id AND old.display_name <> $2
                RETURNING old.display_name",
                id.as_uuid(),
                name,
            )
        )?)
    }

    /// Moves the account's last handle change `days` into the past, so tests need
    /// not wait out the rename cooldown. An account whose handle never changed
    /// has no change to move, and stays as it is.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    #[cfg(any(test, feature = "test-util"))]
    pub async fn age_handle_change(&mut self, id: UserId, days: i32) -> Result<(), DatabaseError> {
        run!(
            self,
            execute,
            sqlx::query!(
                "UPDATE users SET handle_changed_at = handle_changed_at - make_interval(days => $2)
                WHERE id = $1",
                id.as_uuid(),
                days,
            )
        )?;
        Ok(())
    }

    /// Changes the account's status. Returns `false` if it already had it. The
    /// account keeps its verified emails whatever its status.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn set_status(
        &mut self,
        id: UserId,
        status: AccountStatus,
    ) -> Result<bool, DatabaseError> {
        run!(
            self,
            changed_one,
            sqlx::query!(
                "UPDATE users
                SET status = $2, updated_at = now()
                WHERE id = $1
                AND status <> $2",
                id.as_uuid(),
                status.as_str(),
            )
        )
    }

    /// Counts every account, whatever its status.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn count(&mut self) -> Result<u64, DatabaseError> {
        Ok(run!(
            self,
            fetch_one,
            sqlx::query_scalar!(r#"SELECT count(*) AS "count!: Unsigned" FROM users"#)
        )?
        .into())
    }

    /// Lists the `limit` newest accounts, newest first.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or a row is invalid.
    pub async fn newest(&mut self, limit: u16) -> Result<Vec<User>, DatabaseError> {
        run!(
            self,
            fetch_all,
            users!(
                admin "ORDER BY u.created_at DESC, u.id DESC
                LIMIT $1",
                i64::from(limit),
            )
        )
    }

    /// Counts the accounts [`Self::search`] finds for `query`.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn count_matching(&mut self, query: &str) -> Result<u64, DatabaseError> {
        let pattern = Self::escaped(query);
        Ok(run!(
            self,
            fetch_one,
            sqlx::query_scalar!(
                r#"SELECT count(*) AS "count!: Unsigned"
                FROM users u
                JOIN handles h ON h.user_id = u.id AND h.released_at IS NULL
                WHERE (
                    $1 = ''
                    OR u.id::text = lower($1)
                    OR h.handle ILIKE ('%' || $2 || '%')
                    OR u.display_name ILIKE ('%' || $2 || '%')
                    OR EXISTS (
                        SELECT FROM user_emails e
                        WHERE e.user_id = u.id
                        AND e.normalized_email ILIKE ('%' || $2 || '%')
                    )
                )"#,
                query,
                pattern,
            )
        )?
        .into())
    }

    /// Lists up to `limit` accounts whose ID is `query`, or whose handle,
    /// display name or any email contains it, ignoring case throughout, skipping the first
    /// `offset` matches. An empty query matches every account. The closest
    /// matches come first: the account whose ID or handle is the query, then the
    /// one whose email is, then handles starting with it, then display names
    /// with a word starting with it, then the rest; each by handle.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or a row is invalid.
    pub async fn search(
        &mut self,
        query: &str,
        offset: u64,
        limit: u16,
    ) -> Result<Vec<User>, DatabaseError> {
        let pattern = Self::escaped(query);
        run!(
            self,
            fetch_all,
            users!(
                admin "WHERE (
                    $1 = ''
                    OR u.id::text = lower($1)
                    OR h.handle ILIKE ('%' || $2 || '%')
                    OR u.display_name ILIKE ('%' || $2 || '%')
                    OR EXISTS (
                        SELECT FROM user_emails e
                        WHERE e.user_id = u.id
                        AND e.normalized_email ILIKE ('%' || $2 || '%')
                    )
                )
                ORDER BY
                    CASE
                        WHEN $1 = '' THEN 0
                        WHEN u.id::text = lower($1) OR h.handle = lower($1) THEN 0
                        WHEN EXISTS (
                            SELECT FROM user_emails e
                            WHERE e.user_id = u.id AND e.normalized_email = lower($1)
                        ) THEN 1
                        WHEN h.handle ILIKE ($2 || '%') THEN 2
                        WHEN u.display_name ILIKE ($2 || '%')
                            OR u.display_name ILIKE ('% ' || $2 || '%') THEN 3
                        ELSE 4
                    END,
                    h.handle, u.id
                LIMIT $3 OFFSET $4",
                query,
                pattern,
                i64::from(limit),
                i64::try_from(offset).unwrap_or(i64::MAX),
            )
        )
    }

    /// `query` for a `LIKE` pattern, matching itself literally: its wildcards
    /// are escaped.
    fn escaped(query: &str) -> String {
        query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    }
}
impl UserRepository<'_> {
    /// Whether the account `id` takes part in usage statistics: unless its owner
    /// left them (`analytics_opt_outs`).
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn analytics_included(&mut self, id: UserId) -> Result<bool, DatabaseError> {
        Ok(self.analytics_left_at(id).await?.is_none())
    }

    /// When the account `id`'s owner left usage statistics, or `None` while it
    /// takes part.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn analytics_left_at(
        &mut self,
        id: UserId,
    ) -> Result<Option<DateTime<Utc>>, DatabaseError> {
        run!(
            self,
            fetch_optional,
            sqlx::query_scalar!(
                "SELECT opted_out_at FROM analytics_opt_outs WHERE user_id = $1",
                id.as_uuid(),
            )
        )
    }

    /// Includes the account `id` in usage statistics, or leaves it out. Returns
    /// whether that changed anything.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn set_analytics_included(
        &mut self,
        id: UserId,
        included: bool,
    ) -> Result<bool, DatabaseError> {
        let changed = if included {
            run!(
                self,
                execute,
                sqlx::query!(
                    "DELETE FROM analytics_opt_outs WHERE user_id = $1",
                    id.as_uuid(),
                )
            )?
        } else {
            run!(
                self,
                execute,
                sqlx::query!(
                    "INSERT INTO analytics_opt_outs (user_id) VALUES ($1) ON CONFLICT DO NOTHING",
                    id.as_uuid(),
                )
            )?
        };
        Ok(changed.rows_affected() > 0)
    }
}

/// Focused tests for this module and its security contracts.
#[cfg(test)]
mod tests;

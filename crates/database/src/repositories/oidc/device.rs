//! Device authorizations (RFC 8628): the two codes of each, the user's decision
//! on auth's device page, and the device's polls of the token endpoint.

use crate::{
    DatabaseError,
    repositories::{JsonArray, OidcRepository, RepositoryExecutor},
};
use chrono::{DateTime, Utc};
use sourceorigin_common::{ClientId, SessionId, TokenId, UserId};
use sqlx::types::Json;

/// A new device authorization, waiting for its user's decision.
#[derive(Clone, Debug)]
pub struct InsertDeviceAuthorization {
    /// Authorization identifier, which the device code names.
    pub id: TokenId,
    /// Digest of the device code's secret.
    pub secret_digest: Vec<u8>,
    /// Digest of the user code.
    pub user_code_digest: Vec<u8>,
    /// Fingerprint of the digest key that made both digests.
    pub digest_key: String,
    /// The client whose device asks.
    pub client_id: ClientId,
    /// Requested scopes.
    pub scopes: Vec<String>,
    /// Requested token audience.
    pub audience: String,
    /// Seconds the device must leave between two polls.
    pub poll_interval: u16,
    /// When both codes stop being accepted.
    pub expires_at: DateTime<Utc>,
}

/// A device authorization its user has not decided on, as auth's device page
/// shows it. Whether its client's current policy still allows it is the
/// caller's to check.
#[derive(Clone, Debug)]
pub struct PendingDeviceAuthorization {
    /// Authorization identifier.
    pub id: TokenId,
    /// The client whose device asks.
    pub client_id: ClientId,
    /// The client's current consent-screen name.
    pub client_display_name: String,
    /// The account that registered the client; `None` for an operator's client.
    pub client_owner_id: Option<UserId>,
    /// Requested scopes.
    pub scopes: Vec<String>,
    /// Requested token audience.
    pub audience: String,
    /// When the device asked.
    pub created_at: DateTime<Utc>,
    /// When its codes stop being accepted.
    pub expires_at: DateTime<Utc>,
}

/// A [`query_record!`] returning [`PendingDeviceAuthorization`]s, read as `d`,
/// with their client as `c`.
macro_rules! pending_device_authorizations {
    ($tail:literal $(, $arg:expr)* $(,)?) => {
        query_record!(
            PendingDeviceAuthorization,
            r#"d.id AS "id: TokenId", d.client_id AS "client_id: ClientId",
            c.display_name AS client_display_name,
            c.owner_id AS "client_owner_id: UserId",
            d.scopes AS "scopes: JsonArray<String>", d.audience, d.created_at, d.expires_at"#,
            "SELECT",
            "FROM device_authorizations d
            JOIN oidc_clients c ON c.id = d.client_id
            WHERE d.user_code_digest = ANY($1)
            AND d.status = 'pending'
            AND d.expires_at > now()
            AND c.enabled " + $tail
            $(, $arg)*
        )
    };
}

sourceorigin_common::str_enum! {
    /// A device authorization's stored status.
    enum DeviceStatus {
        /// Its user has not decided.
        Pending = "pending",
        /// Its user approved it, and the device has not collected its tokens.
        Approved = "approved",
        /// Its user refused it.
        Denied = "denied",
        /// The device collected its tokens.
        Redeemed = "redeemed",
    }
}

/// What a device authorization's user decided, and whether the device has
/// collected its tokens. An approval carries who approved, so one without an
/// account cannot be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceAuthorizationState {
    /// The user has not decided.
    Pending,
    /// The user approved it, and the device has not collected its tokens.
    Approved {
        /// The account that approved.
        user_id: UserId,
        /// The session the approval created for the device.
        session_id: SessionId,
    },
    /// The user refused it.
    Denied,
    /// The device collected its tokens.
    Redeemed,
}

/// A device authorization as one poll of its device found it.
#[derive(Clone, Debug)]
pub struct PolledDeviceAuthorization {
    /// Authorization identifier.
    pub id: TokenId,
    /// The client the device code was issued to.
    pub client_id: ClientId,
    /// Requested scopes.
    pub scopes: Vec<String>,
    /// Requested token audience.
    pub audience: String,
    /// What its user decided.
    pub state: DeviceAuthorizationState,
    /// Whether its codes are past their expiry.
    pub expired: bool,
    /// Whether this poll came before its interval since the last one had passed.
    pub early: bool,
}

impl OidcRepository<'_> {
    /// Stores a device authorization.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::Conflict`] if another authorization holds the
    /// same user code, or another [`DatabaseError`] if the insert fails.
    pub async fn insert_device_authorization(
        &mut self,
        input: &InsertDeviceAuthorization,
    ) -> Result<(), DatabaseError> {
        run!(
            self,
            execute,
            sqlx::query!(
                "INSERT INTO device_authorizations (
                    id, secret_digest, user_code_digest, digest_key, client_id, scopes,
                    audience, poll_interval, expires_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                input.id.as_uuid(),
                input.secret_digest,
                input.user_code_digest,
                input.digest_key,
                input.client_id.as_str(),
                Json(&input.scopes) as _,
                input.audience,
                i32::from(input.poll_interval),
                input.expires_at,
            )
        )?;
        Ok(())
    }

    /// Finds the undecided, unexpired device authorization of an enabled client
    /// whose user code's digest is any of `digests`, one per accepted digest key.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or the row is invalid.
    pub async fn find_pending_device_authorization(
        &mut self,
        digests: &[Vec<u8>],
    ) -> Result<Option<PendingDeviceAuthorization>, DatabaseError> {
        run!(
            self,
            fetch_optional,
            pending_device_authorizations!("", digests)
        )
    }

    /// Finds a device authorization as [`Self::find_pending_device_authorization`]
    /// does, and locks it until the transaction ends, so its user's decision and
    /// its device's polls take turns.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::OutsideTransaction`] outside a transaction, or
    /// another [`DatabaseError`] if the query fails or the row is invalid.
    pub async fn lock_pending_device_authorization(
        &mut self,
        digests: &[Vec<u8>],
    ) -> Result<Option<PendingDeviceAuthorization>, DatabaseError> {
        self.executor.require_transaction()?;
        run!(
            self,
            fetch_optional,
            pending_device_authorizations!("FOR UPDATE OF d", digests)
        )
    }

    /// Records the user's approval of the undecided device authorization `id`:
    /// `user_id` approved, which created `session_id` for the device.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::NotFound`] if it is no longer undecided, or
    /// another [`DatabaseError`] if the query fails.
    pub async fn approve_device_authorization(
        &mut self,
        id: TokenId,
        user_id: UserId,
        session_id: SessionId,
    ) -> Result<(), DatabaseError> {
        run!(
            self,
            required,
            sqlx::query!(
                "UPDATE device_authorizations
                SET status = 'approved', user_id = $2, session_id = $3
                WHERE id = $1 AND status = 'pending'",
                id.as_uuid(),
                user_id.as_uuid(),
                session_id.as_uuid(),
            )
        )
    }

    /// Records the user's refusal of the undecided device authorization `id`.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::NotFound`] if it is no longer undecided, or
    /// another [`DatabaseError`] if the query fails.
    pub async fn deny_device_authorization(&mut self, id: TokenId) -> Result<(), DatabaseError> {
        run!(
            self,
            required,
            sqlx::query!(
                "UPDATE device_authorizations SET status = 'denied'
                WHERE id = $1 AND status = 'pending'",
                id.as_uuid(),
            )
        )
    }

    /// The account that approved the device authorization `id`, whose device
    /// code's digest is any of `digests`, read without a lock: `None` if no such
    /// authorization exists, or nobody approved it.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or the row is invalid.
    pub async fn device_authorization_user(
        &mut self,
        id: TokenId,
        digests: &[Vec<u8>],
    ) -> Result<Option<UserId>, DatabaseError> {
        Ok(run!(
            self,
            fetch_optional,
            sqlx::query_scalar!(
                r#"SELECT user_id AS "user_id: UserId"
                FROM device_authorizations
                WHERE id = $1 AND secret_digest = ANY($2)"#,
                id.as_uuid(),
                digests,
            )
        )?
        .flatten())
    }

    /// Records one poll of the device authorization `id`, whose device code's
    /// digest is any of `digests`, and returns it as the poll found it, locked
    /// until the transaction ends. A poll of an undecided, unexpired
    /// authorization that comes before its interval has passed adds `slow_down`
    /// seconds to that interval, for this poll and every later one. `None` if no
    /// such authorization exists.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::OutsideTransaction`] outside a transaction, or
    /// another [`DatabaseError`] if the query fails or the row is invalid.
    pub async fn poll_device_authorization(
        &mut self,
        id: TokenId,
        digests: &[Vec<u8>],
        slow_down: u16,
    ) -> Result<Option<PolledDeviceAuthorization>, DatabaseError> {
        self.executor.require_transaction()?;
        let Some(row) = run!(
            self,
            fetch_optional,
            sqlx::query!(
                r#"WITH polled AS (
                    SELECT id, COALESCE(
                        last_polled_at + poll_interval * interval '1 second' > now(), FALSE
                    ) AS early
                    FROM device_authorizations
                    WHERE id = $1 AND secret_digest = ANY($2)
                    FOR UPDATE
                )
                UPDATE device_authorizations d
                SET last_polled_at = now(),
                    poll_interval = d.poll_interval + CASE
                        WHEN polled.early AND d.status = 'pending' AND d.expires_at > now()
                        THEN $3::int ELSE 0
                    END
                FROM polled
                WHERE d.id = polled.id
                RETURNING
                    d.id AS "id: TokenId", d.client_id AS "client_id: ClientId",
                    d.scopes AS "scopes: JsonArray<String>", d.audience,
                    d.status AS "status: DeviceStatus",
                    d.user_id AS "user_id: UserId", d.session_id AS "session_id: SessionId",
                    d.expires_at <= now() AS "expired!", polled.early AS "early!""#,
                id.as_uuid(),
                digests,
                i32::from(slow_down),
            )
        )?
        else {
            return Ok(None);
        };
        let state = match (row.status, row.user_id.zip(row.session_id)) {
            (DeviceStatus::Pending, _) => DeviceAuthorizationState::Pending,
            (DeviceStatus::Denied, _) => DeviceAuthorizationState::Denied,
            (DeviceStatus::Redeemed, _) => DeviceAuthorizationState::Redeemed,
            (DeviceStatus::Approved, Some((user_id, session_id))) => {
                DeviceAuthorizationState::Approved {
                    user_id,
                    session_id,
                }
            }
            (DeviceStatus::Approved, None) => {
                return Err(DatabaseError::InvalidStoredData {
                    column: "status".to_owned(),
                    source: "an approved device authorization names its account and session".into(),
                });
            }
        };
        Ok(Some(PolledDeviceAuthorization {
            id: row.id,
            client_id: row.client_id,
            scopes: row.scopes.into(),
            audience: row.audience,
            state,
            expired: row.expired,
            early: row.early,
        }))
    }

    /// Marks the approved device authorization `id` as redeemed, so its device
    /// code works once.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::NotFound`] if it is not approved, or another
    /// [`DatabaseError`] if the query fails.
    pub async fn redeem_device_authorization(&mut self, id: TokenId) -> Result<(), DatabaseError> {
        run!(
            self,
            required,
            sqlx::query!(
                "UPDATE device_authorizations SET status = 'redeemed'
                WHERE id = $1 AND status = 'approved'",
                id.as_uuid(),
            )
        )
    }

    /// Makes every device authorization of `client` expire now: a fixture for
    /// expiry.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the update fails.
    #[cfg(any(test, feature = "test-util"))]
    pub async fn expire_device_authorizations(
        &mut self,
        client: &ClientId,
    ) -> Result<(), DatabaseError> {
        run!(
            self,
            execute,
            sqlx::query!(
                "UPDATE device_authorizations SET expires_at = clock_timestamp()
                WHERE client_id = $1",
                client.as_str(),
            )
        )?;
        Ok(())
    }

    /// Records that `client` received tokens from `session`, as redeeming a code
    /// does through its trigger, so the client is told when the session ends.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn record_session_client(
        &mut self,
        session: SessionId,
        client: &ClientId,
    ) -> Result<(), DatabaseError> {
        run!(
            self,
            execute,
            sqlx::query!(
                "INSERT INTO oidc_session_clients (session_id, client_id)
                VALUES ($1, $2)
                ON CONFLICT DO NOTHING",
                session.as_uuid(),
                client.as_str(),
            )
        )?;
        Ok(())
    }
}

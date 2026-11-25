//! Bounded deletion of expired ephemeral authentication state, event personal data,
//! finished deliveries and the records an account no longer uses, and what is
//! left of the digests of opaque tokens.

use super::{MaintenanceRepository, RepositoryExecutor, Unsigned};
use crate::DatabaseError;

sourceorigin_common::str_enum! {
    /// The records that store an opaque token's secret as a keyed digest.
    pub enum DigestedSecret {
        /// A pending authorization request.
        AuthorizationRequest = "authorization_request",
        /// An authorization code.
        AuthorizationCode = "authorization_code",
        /// A refresh token.
        RefreshToken = "refresh_token",
    }
}

/// How many stored digests of one kind a digest key made.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DigestUsage {
    /// The kind of record.
    pub secret: DigestedSecret,
    /// The digest key's fingerprint.
    pub digest_key: String,
    /// How many of them are stored.
    pub stored: u64,
}

/// Maximum rows removed from each table in one maintenance batch.
const MAX_PURGE_BATCH: u32 = 1_000;

impl MaintenanceRepository<'_> {
    /// Counts stored digests per kind and digest key. Every stored one counts,
    /// expired and consumed ones too: cleanup deletes each digest once no lookup
    /// can match it, and until then one may still be presented, e.g. as a code
    /// replay that revokes the grant it produced. A digest key can be dropped
    /// once it made none.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or a row is invalid.
    pub async fn digest_usage(&mut self) -> Result<Vec<DigestUsage>, DatabaseError> {
        run!(
            self,
            fetch_all,
            sqlx::query_as!(
                DigestUsage,
                r#"SELECT
                    secret AS "secret!: DigestedSecret", digest_key AS "digest_key!",
                    count(*) AS "stored!: Unsigned"
                FROM (
                    SELECT 'authorization_request' AS secret, digest_key
                    FROM authorization_requests
                    UNION ALL
                    SELECT 'authorization_code', digest_key
                    FROM authorization_codes
                    UNION ALL
                    SELECT 'refresh_token', digest_key
                    FROM refresh_tokens
                ) digests
                GROUP BY secret, digest_key
                ORDER BY secret, digest_key"#
            )
        )
    }

    /// Purges a bounded batch of expired ephemeral authentication state, of known devices unused
    /// for 400 days, of event personal data older than 90 days, of deliveries finished over 7 or
    /// 30 days ago, and of what an account no longer uses (revoked passkeys, unlinked identities
    /// and ended recoveries over 90 days old, announcements and notifications over a year old,
    /// and the handles of accounts purged over a year ago). Returns how many rows were deleted;
    /// `batch_size` is clamped to 1..=1000 rows per table. Outside a transaction each table's
    /// batch commits on its own.
    ///
    /// # Errors
    /// Returns [`DatabaseError`] if a delete fails.
    #[expect(
        clippy::too_many_lines,
        reason = "Each purge statement is inline SQL so that it is checked at compile time."
    )]
    pub async fn purge_expired_auth_state(
        &mut self,
        batch_size: u32,
    ) -> Result<u64, DatabaseError> {
        let limit = i64::from(batch_size.clamp(1, MAX_PURGE_BATCH));
        let mut purged = 0;
        macro_rules! purge {
            ($table:literal where $condition:literal) => {
                purge!(
                    "DELETE FROM "
                        + $table
                        + " WHERE ctid IN (SELECT ctid FROM "
                        + $table
                        + " WHERE "
                        + $condition
                        + " LIMIT $1 FOR UPDATE SKIP LOCKED)"
                )
            };
            ($statement:literal $(+ $more:literal)*) => {
                purged += run!(self, execute, sqlx::query!($statement $(+ $more)*, limit))?
                    .rows_affected()
            };
        }
        purge!("request_limits" where "expires_at < now()");
        purge!("invitations" where "expires_at < now()
            OR redeemed_at < now() - interval '1 day'");
        purge!(
            "DELETE FROM user_emails
            WHERE id IN (
                SELECT secret.email_id
                FROM email_verification_secrets secret
                JOIN user_emails email ON email.id = secret.email_id
                WHERE secret.expires_at < now()
                AND email.verified_at IS NULL
                LIMIT $1
                FOR UPDATE OF secret, email SKIP LOCKED
            )"
        );
        purge!("pending_federation" where "expires_at < now()");
        purge!("federation_transactions" where "expires_at < now()
            OR consumed_at < now() - interval '1 day'");
        purge!("webauthn_ceremonies" where "expires_at < now()
            OR consumed_at < now() - interval '1 day'");
        purge!("dashboard_handoffs" where "expires_at < now()");
        purge!("dashboard_sessions" where "expires_at < now()");
        purge!("authorization_requests" where "expires_at < now()
            OR consumed_at < now() - interval '1 day'");
        purge!("device_authorizations" where "expires_at < now() - interval '1 hour'");
        purge!(
            "DELETE FROM authorization_codes c
            WHERE c.ctid IN (
                SELECT code.ctid
                FROM authorization_codes code
                WHERE (code.consumed_at IS NULL AND code.expires_at < now())
                OR (
                    code.consumed_at < now() - interval '1 day'
                    AND NOT EXISTS (
                        SELECT 1
                        FROM refresh_token_families family
                        JOIN sessions session ON session.id = family.session_id
                        WHERE family.authorization_code_id = code.id
                        AND family.revoked_at IS NULL
                        AND session.revoked_at IS NULL
                        AND session.expires_at > now()
                    )
                )
                LIMIT $1
                FOR UPDATE OF code SKIP LOCKED
            )"
        );
        purge!(
            "DELETE FROM refresh_tokens t
            WHERE t.ctid IN (
                SELECT token.ctid
                FROM refresh_tokens token
                JOIN refresh_token_families family ON family.id = token.family_id
                WHERE token.expires_at < now()
                OR family.revoked_at < now() - interval '1 day'
                LIMIT $1
                FOR UPDATE OF token SKIP LOCKED
            )"
        );
        purge!(
            "DELETE FROM refresh_token_families f
            WHERE f.ctid IN (
                SELECT family.ctid
                FROM refresh_token_families family
                WHERE (
                    family.revoked_at < now() - interval '1 day'
                    OR family.created_at < now() - interval '31 days'
                )
                AND NOT EXISTS (
                    SELECT 1
                    FROM refresh_tokens token
                    WHERE token.family_id = family.id
                )
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )"
        );
        purge!(
            "DELETE FROM browser_session_secrets b
            WHERE b.ctid IN (
                SELECT secret.ctid
                FROM browser_session_secrets secret
                JOIN sessions session ON session.id = secret.session_id
                WHERE session.expires_at < now()
                OR session.revoked_at < now() - interval '1 day'
                LIMIT $1
                FOR UPDATE OF secret SKIP LOCKED
            )"
        );
        purge!("known_devices" where "last_seen_at < now() - interval '400 days'");
        purge!("event_personal_data" where "created_at < now() - interval '90 days'");
        purge!("passkey_credentials" where "revoked_at < now() - interval '90 days'");
        purge!("external_login_identities" where "revoked_at < now() - interval '90 days'");
        purge!("account_recoveries" where "CASE WHEN completed_at IS NULL THEN least(expires_at, cancelled_at)
            ELSE least(undone_at, settles_at) END < now() - interval '90 days'");
        purge!("notifications" where "created_at < now() - interval '1 year'");
        purge!(
            "DELETE FROM announcement_recipients r
            WHERE r.ctid IN (
                SELECT recipient.ctid
                FROM announcement_recipients recipient
                JOIN announcements a ON a.id = recipient.announcement_id
                WHERE a.sent_at < now() - interval '1 year'
                LIMIT $1
                FOR UPDATE OF recipient SKIP LOCKED
            )"
        );
        purge!(
            "DELETE FROM announcements a
            WHERE a.ctid IN (
                SELECT announcement.ctid
                FROM announcements announcement
                WHERE NOT announcement.everyone
                AND announcement.sent_at < now() - interval '1 year'
                AND NOT EXISTS (
                    SELECT FROM announcement_recipients r
                    WHERE r.announcement_id = announcement.id
                )
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )"
        );
        purge!("handles" where "user_id IS NULL AND released_at < now() - interval '1 year'");
        purge!(
            "DELETE FROM deliveries d
            WHERE d.ctid IN (
                SELECT delivery.ctid
                FROM deliveries delivery
                WHERE (delivery.status = 'succeeded' AND delivery.finished_at < now() - interval '7 days')
                OR (
                    delivery.status IN ('failed', 'cancelled')
                    AND delivery.finished_at < now() - interval '30 days'
                )
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )"
        );
        purge!(
            "DELETE FROM sessions s
            WHERE s.ctid IN (
                SELECT session.ctid
                FROM sessions session
                WHERE (
                    session.expires_at < now()
                    OR session.revoked_at < now() - interval '1 day'
                )
                AND NOT EXISTS (
                    SELECT 1
                    FROM refresh_token_families family
                    WHERE family.session_id = session.id
                )
                AND NOT EXISTS (
                    SELECT 1
                    FROM authorization_codes code
                    WHERE code.session_id = session.id
                )
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )"
        );
        Ok(purged)
    }
}

/// Retention of the records an account no longer uses.
#[cfg(test)]
mod tests;

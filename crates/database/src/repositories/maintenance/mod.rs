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

    /// Purges a bounded batch of expired ephemeral authentication state, of known
    /// devices unused for 400 days, of event personal data older than 90 days, of
    /// deliveries that succeeded over 7 days ago or failed or were cancelled over
    /// 30 days ago, and of what an
    /// account no longer uses: passkeys revoked, identities unlinked and
    /// recoveries ended over 90 days ago, notifications and the recipients of
    /// announcements sent over a year ago, with the announcements to chosen
    /// accounts they leave without one, and the handles of accounts purged over a
    /// year ago, which anyone may then claim.
    /// Returns how many rows were deleted. `batch_size` is clamped to 1..=1000
    /// rows per table.
    ///
    /// Each batch skips rows that live requests have locked, except event personal
    /// data, which requests never lock. Outside a transaction,
    /// each table's batch commits on its own, so cleanup never holds locks across
    /// tables; inside one, all of them commit or roll back with it. Deleting a
    /// session or an authorization code can still wait briefly on a locked child row
    /// it cascades to or unlinks.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if a delete fails. Outside a transaction, earlier
    /// tables stay purged.
    #[expect(
        clippy::too_many_lines,
        reason = "Each purge statement is inline SQL so that it is checked at compile time."
    )]
    pub async fn purge_expired_auth_state(
        &mut self,
        batch_size: u32,
    ) -> Result<u64, DatabaseError> {
        let limit = i64::from(batch_size.clamp(1, MAX_PURGE_BATCH));
        // Runs each statement in order, each removing at most `$1` expired rows
        // that no live request has locked, and adds up what they removed. A
        // checked query needs a literal, and each has its own type, so the
        // statements cannot share a loop. Most delete the rows of one table that
        // match a condition, which `$table where $condition` writes.
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
        // An unverified address goes with its expired verification link; the
        // secret's row cascades. Both rows are locked, skipping any a
        // verification holds, so cleanup never waits on one.
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
        // A session that went unused too long has ended too, but stays until
        // its `expires_at`, which comes soon after.
        purge!("dashboard_sessions" where "expires_at < now()");
        purge!("authorization_requests" where "expires_at < now()
            OR consumed_at < now() - interval '1 day'");
        // A redeemed code stays while a replay could still revoke something: an
        // unrevoked family it produced whose session is live (refresh tokens expire
        // with the session). The first day after redemption is kept regardless, so
        // replays are still recorded even for codes that produced no refresh family.
        // Retained codes are rechecked on every pass, so this scan grows with
        // every redeemed code past its first day, not only with purgeable ones.
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
        // A browser unused for 400 days has lost its device cookie, which
        // lives as long: its next sign-in is a new device's either way.
        purge!("known_devices" where "last_seen_at < now() - interval '400 days'");
        // Events are kept indefinitely; the personal data beside them is not.
        // A purge erasing its apps' origins may hold some of these rows.
        purge!("event_personal_data" where "created_at < now() - interval '90 days'");
        // An account's ended records go once its history no longer shows their
        // events' details, 90 days on, as event personal data does.
        purge!("passkey_credentials" where "revoked_at < now() - interval '90 days'");
        purge!("external_login_identities" where "revoked_at < now() - interval '90 days'");
        // A recovery ends when it is undone or its probation settles, or,
        // unused, when its link is cancelled or expires, whichever comes first:
        // cancelling an expired link, as the next start does, ends nothing.
        purge!("account_recoveries" where "CASE WHEN completed_at IS NULL THEN least(expires_at, cancelled_at)
            ELSE least(undone_at, settles_at) END < now() - interval '90 days'");
        purge!("notifications" where "created_at < now() - interval '1 year'");
        // Who received an announcement goes with the inbox copies, a year on.
        // One to every account stays, as the administrators' record of what
        // they said; one to chosen accounts may concern them, so it goes
        // with its last recipient, as a purge does.
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
        // A purged account's handles were released at its purge (the
        // `handles_release_purged` trigger), and stay reserved a year, so
        // nobody takes one to pass for it.
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

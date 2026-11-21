-- A verified address belongs to its account until the account is purged, whatever
-- its status. Disabling an account used to release its addresses, so another
-- account could verify one, and enabling it again failed once one had. Now a
-- disabled account keeps them, as an account scheduled for deletion already did,
-- and deleting the account's row at its purge is what releases them.
--
-- `active_verified_email_ownership` becomes `verified_email_ownership`, one row
-- per verified address naming the account that holds it. The rename breaks the
-- expand-then-contract rule: the previous release's lookups of who holds an
-- address fail until its replacement is healthy. There are no real users yet,
-- so that window costs less than keeping the old name around.

DROP TRIGGER users_synchronize_email_ownership ON users;
DROP FUNCTION synchronize_user_email_ownership();
DROP TRIGGER user_emails_claim_active_verified ON user_emails;
DROP FUNCTION claim_active_verified_email();
DROP TRIGGER user_emails_release_active_verified ON user_emails;
DROP FUNCTION release_active_verified_email();

ALTER TABLE active_verified_email_ownership RENAME TO verified_email_ownership;
ALTER INDEX active_verified_email_ownership_pkey RENAME TO verified_email_ownership_pkey;
ALTER INDEX active_verified_email_ownership_user_idx RENAME TO verified_email_ownership_user_idx;
ALTER TABLE verified_email_ownership
    RENAME CONSTRAINT active_verified_email_ownership_user_id_fkey TO verified_email_ownership_user_id_fkey;

-- Claims ownership when an account gains a verified address, and fails as a
-- unique violation if another account holds it.
CREATE FUNCTION claim_verified_email() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF NEW.verified_at IS NOT NULL THEN
        INSERT INTO verified_email_ownership(normalized_email, user_id)
        VALUES (NEW.normalized_email, NEW.user_id)
        ON CONFLICT (normalized_email) DO UPDATE SET user_id = EXCLUDED.user_id
        WHERE verified_email_ownership.user_id = EXCLUDED.user_id;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'verified email already belongs to another account' USING ERRCODE = 'unique_violation';
        END IF;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER user_emails_claim_verified
    BEFORE INSERT OR UPDATE OF normalized_email, verified_at, user_id ON user_emails
    FOR EACH ROW EXECUTE FUNCTION claim_verified_email();

-- Releases ownership once no verified row of the owner still holds the address.
-- AFTER triggers see the updated table, including an unchanged verified row.
CREATE FUNCTION release_verified_email() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF OLD.verified_at IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM user_emails
        WHERE user_id=OLD.user_id AND normalized_email=OLD.normalized_email
          AND verified_at IS NOT NULL
    ) THEN
        DELETE FROM verified_email_ownership
        WHERE normalized_email=OLD.normalized_email AND user_id=OLD.user_id;
    END IF;
    RETURN OLD;
END $$;
CREATE TRIGGER user_emails_release_verified
    AFTER DELETE OR UPDATE OF normalized_email, verified_at, user_id ON user_emails
    FOR EACH ROW EXECUTE FUNCTION release_verified_email();

-- Disabled accounts own nothing yet, so an address may be verified on several
-- accounts. One keeps it: the account that owns it, otherwise the one that
-- verified it last. The others' rows become unverified and stop being primary;
-- their owners can remove them, or verify them once the address is free.
WITH keepers AS (
    SELECT DISTINCT ON (e.normalized_email) e.normalized_email, e.user_id
    FROM user_emails e
    LEFT JOIN verified_email_ownership o
        ON o.normalized_email = e.normalized_email AND o.user_id = e.user_id
    WHERE e.verified_at IS NOT NULL
    ORDER BY e.normalized_email, o.user_id IS NOT NULL DESC, e.verified_at DESC, e.id DESC
)
UPDATE user_emails e SET verified_at = NULL, is_primary = FALSE
FROM keepers k
WHERE e.normalized_email = k.normalized_email
  AND e.user_id <> k.user_id
  AND e.verified_at IS NOT NULL;

-- An account that lost its primary keeps its other verified addresses, so its
-- earliest verified one becomes primary, as `user_emails_exactly_one_primary`
-- requires at commit.
UPDATE user_emails SET is_primary = TRUE
WHERE id IN (
    SELECT DISTINCT ON (e.user_id) e.id
    FROM user_emails e
    WHERE e.verified_at IS NOT NULL
      AND NOT EXISTS (
          SELECT 1 FROM user_emails p
          WHERE p.user_id = e.user_id AND p.is_primary AND p.verified_at IS NOT NULL
      )
    ORDER BY e.user_id, e.verified_at, e.id
);

-- Every remaining verified address now has one account: record the ones disabled
-- accounts verified, which nothing owned.
INSERT INTO verified_email_ownership (normalized_email, user_id)
SELECT DISTINCT normalized_email, user_id FROM user_emails WHERE verified_at IS NOT NULL
ON CONFLICT (normalized_email) DO NOTHING;

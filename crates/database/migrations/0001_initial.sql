-- The authentication schema. Released migrations are immutable; later schema
-- changes go in new numbered files.

----------------------------------------------------------------------------
-- Accounts
----------------------------------------------------------------------------

-- A 'deleting' account waits out its grace period in account_deletions.
-- handle_changed_at is NULL until the handle first changes: a new account may
-- correct the handle it chose at signup at once, and the rename cooldown counts
-- only from a change.
CREATE TABLE users (
    id UUID PRIMARY KEY,
    display_name TEXT NOT NULL,
    profile_version BIGINT NOT NULL DEFAULT 1 CHECK (profile_version > 0),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled', 'deleting')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    handle_changed_at TIMESTAMPTZ
);

-- The confusable skeleton `Handle::skeleton` computes, which a test pins to it:
-- hyphens dropped, `0` read as `o`, `1` and `i` as `l`, `5` as `s`, then `rn` as
-- `m` and `vv` as `w`.
CREATE FUNCTION handle_skeleton(handle TEXT) RETURNS TEXT
    LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
    SET search_path = pg_catalog, public, pg_temp
    RETURN replace(replace(translate(handle, '01i5-', 'olls'), 'rn', 'm'), 'vv', 'w');

-- One table holds every account's current handle, the ones it held before and
-- the ones of purged accounts, so PostgreSQL keeps all of them unique, along with
-- their look-alikes. Only its former owner may claim a released handle again,
-- until an operator deletes it, or, for a purged account's, until cleanup frees
-- it a year after the purge.
CREATE TABLE handles (
    handle TEXT PRIMARY KEY
        CHECK (length(handle) BETWEEN 3 AND 32 AND handle ~ '^[a-z0-9]+(-[a-z0-9]+)*$'),
    skeleton TEXT NOT NULL UNIQUE GENERATED ALWAYS AS (handle_skeleton(handle)) STORED,
    -- NULL once the account is purged.
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    claimed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- NULL while it is the account's current handle.
    released_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX handles_current_idx ON handles (user_id) WHERE released_at IS NULL;
CREATE INDEX handles_user_idx ON handles (user_id);
CREATE INDEX handles_purged_idx ON handles (released_at) WHERE user_id IS NULL;

-- Purging an account sets its handles' user_id to NULL, which releases them all
-- at the purge, overwriting when an older one was renamed away, so each is freed
-- a year after the purge.
CREATE FUNCTION release_purged_handle() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    NEW.released_at := now();
    RETURN NEW;
END $$;
CREATE TRIGGER handles_release_purged
    BEFORE UPDATE OF user_id ON handles
    FOR EACH ROW WHEN (OLD.user_id IS NOT NULL AND NEW.user_id IS NULL)
    EXECUTE FUNCTION release_purged_handle();

-- Handles administrators reserve, on top of the names the handle rules build in:
-- signup and the settings refuse them and their look-alikes, as they do built-in
-- ones, until an administrator gives one to an account or removes it from the
-- list. The reason says who or what it is kept for. Accounts already holding one
-- keep it, and `handle check` lists them.
CREATE TABLE reserved_handles (
    handle TEXT PRIMARY KEY
        CHECK (length(handle) BETWEEN 3 AND 32 AND handle ~ '^[a-z0-9]+(-[a-z0-9]+)*$'),
    skeleton TEXT NOT NULL UNIQUE GENERATED ALWAYS AS (handle_skeleton(handle)) STORED,
    reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
    reserved_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Each account's profile picture: the JPEG auth re-encoded from the upload, never
-- the uploaded bytes. `id` names it in its URL and is new on every upload, so the
-- image at a URL never changes and caches keep it for good; replacing or removing
-- the picture stops that URL at once.
CREATE TABLE profile_pictures (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    id UUID NOT NULL UNIQUE,
    image BYTEA NOT NULL CHECK (octet_length(image) BETWEEN 1 AND 131072),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- JPEG does not compress further: keep images out of line without trying.
ALTER TABLE profile_pictures ALTER COLUMN image SET STORAGE EXTERNAL;

-- Administrator accounts. A separate table, rather than a role column, keeps
-- administrator rights out of tokens and out of a future groups model. Who
-- granted the rights is recorded by the AdministratorGranted event.
CREATE TABLE administrators (
    user_id UUID PRIMARY KEY REFERENCES users(id),
    granted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

----------------------------------------------------------------------------
-- Email addresses
----------------------------------------------------------------------------

CREATE TABLE user_emails (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    normalized_email TEXT NOT NULL,
    verified_at TIMESTAMPTZ,
    is_primary BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (NOT is_primary OR verified_at IS NOT NULL)
);
CREATE INDEX user_emails_user_id_idx ON user_emails (user_id);
CREATE INDEX user_emails_verified_lookup_idx ON user_emails (normalized_email) WHERE verified_at IS NOT NULL;
CREATE UNIQUE INDEX user_emails_one_primary_verified_idx ON user_emails (user_id) WHERE is_primary AND verified_at IS NOT NULL;

-- One row per verified address of an active or deleting account. The normalized
-- address is the unique lockable key, which makes concurrent first claims safe.
-- Triggers below keep it in sync with user_emails and users.status. A 'deleting'
-- account keeps its addresses, so restoring it can never collide; deleting the
-- row at its purge releases them.
CREATE TABLE active_verified_email_ownership (
    normalized_email TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE
);
CREATE INDEX active_verified_email_ownership_user_idx ON active_verified_email_ownership (user_id);

-- Claims ownership when an active account gains a verified address.
CREATE FUNCTION claim_active_verified_email() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
DECLARE account_active BOOLEAN;
BEGIN
    IF NEW.verified_at IS NOT NULL THEN
        -- Serialize email claims with status changes. Without this row lock an
        -- insert can observe an active account, wait behind disablement, and
        -- recreate ownership after the account has become disabled. NO KEY UPDATE
        -- still admits foreign-key checks, which FOR UPDATE would deadlock with.
        SELECT status = 'active' INTO account_active
        FROM users WHERE id = NEW.user_id FOR NO KEY UPDATE;
        IF account_active THEN
            INSERT INTO active_verified_email_ownership(normalized_email, user_id)
            VALUES (NEW.normalized_email, NEW.user_id)
            ON CONFLICT (normalized_email) DO UPDATE SET user_id = EXCLUDED.user_id
            WHERE active_verified_email_ownership.user_id = EXCLUDED.user_id;
            IF NOT FOUND THEN
                RAISE EXCEPTION 'verified email already belongs to an active account' USING ERRCODE = 'unique_violation';
            END IF;
        END IF;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER user_emails_claim_active_verified
    BEFORE INSERT OR UPDATE OF normalized_email, verified_at, user_id ON user_emails
    FOR EACH ROW EXECUTE FUNCTION claim_active_verified_email();

-- Releases ownership once no verified row of the owner still holds the address.
-- AFTER triggers see the updated table, including an unchanged verified row.
CREATE FUNCTION release_active_verified_email() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF OLD.verified_at IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM user_emails e JOIN users u ON u.id=e.user_id
        WHERE e.user_id=OLD.user_id AND e.normalized_email=OLD.normalized_email
          AND e.verified_at IS NOT NULL AND u.status='active'
    ) THEN
        DELETE FROM active_verified_email_ownership
        WHERE normalized_email=OLD.normalized_email AND user_id=OLD.user_id;
    END IF;
    RETURN OLD;
END $$;
CREATE TRIGGER user_emails_release_active_verified
    AFTER DELETE OR UPDATE OF normalized_email, verified_at, user_id ON user_emails
    FOR EACH ROW EXECUTE FUNCTION release_active_verified_email();

-- Disabling an account releases its addresses; reactivating reclaims them and
-- fails if another active account claimed one in the meantime.
CREATE FUNCTION synchronize_user_email_ownership() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
DECLARE address TEXT;
BEGIN
    IF NEW.status='disabled' THEN
        DELETE FROM active_verified_email_ownership WHERE user_id=NEW.id;
    ELSIF OLD.status='disabled' AND NEW.status='active' THEN
        FOR address IN SELECT DISTINCT normalized_email FROM user_emails WHERE user_id=NEW.id AND verified_at IS NOT NULL LOOP
            INSERT INTO active_verified_email_ownership(normalized_email,user_id) VALUES(address,NEW.id)
            ON CONFLICT(normalized_email) DO UPDATE SET user_id=EXCLUDED.user_id
            WHERE active_verified_email_ownership.user_id=EXCLUDED.user_id;
            IF NOT FOUND THEN
                RAISE EXCEPTION 'verified email already belongs to an active account' USING ERRCODE='unique_violation';
            END IF;
        END LOOP;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER users_synchronize_email_ownership
    BEFORE UPDATE OF status ON users
    FOR EACH ROW EXECUTE FUNCTION synchronize_user_email_ownership();

-- An account with verified emails has exactly one verified primary, checked at commit.
CREATE FUNCTION enforce_primary_verified_email() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
DECLARE
    affected UUID;
    affected_users UUID[];
BEGIN
    IF TG_OP = 'INSERT' THEN affected_users := ARRAY[NEW.user_id];
    ELSIF TG_OP = 'DELETE' THEN affected_users := ARRAY[OLD.user_id];
    ELSE affected_users := ARRAY[NEW.user_id, OLD.user_id];
    END IF;

    FOREACH affected IN ARRAY affected_users LOOP
        IF EXISTS (SELECT 1 FROM user_emails WHERE user_id=affected AND verified_at IS NOT NULL)
           AND (SELECT count(*) FROM user_emails WHERE user_id=affected AND verified_at IS NOT NULL AND is_primary) <> 1
        THEN RAISE EXCEPTION 'verified emails require exactly one primary' USING ERRCODE='check_violation'; END IF;
    END LOOP;
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END $$;
CREATE CONSTRAINT TRIGGER user_emails_exactly_one_primary
    AFTER INSERT OR UPDATE OR DELETE ON user_emails
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION enforce_primary_verified_email();

-- Users add their own email addresses, which stay unverified until someone
-- follows the link sent to them. Auth derives the link's token from the address's
-- ID with a server key, so the token itself is stored nowhere: only its SHA-256
-- digest, to look the address up. Verifying deletes the row; cleanup deletes
-- expired rows together with the unverified addresses they belong to.
CREATE TABLE email_verification_secrets (
    email_id UUID PRIMARY KEY REFERENCES user_emails(id) ON DELETE CASCADE,
    secret_digest BYTEA NOT NULL UNIQUE CHECK (octet_length(secret_digest) = 32),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX email_verification_secrets_expires_idx ON email_verification_secrets (expires_at);

----------------------------------------------------------------------------
-- Account deletion
----------------------------------------------------------------------------

-- A user, administrator or operator schedules an account's deletion; the account
-- then waits in status 'deleting' for its grace period, during which a passkey
-- can restore it, and the purge job deletes it once the period ends. Its handles
-- stay in `handles` without an owner.
--
-- One row per account scheduled for deletion, removed when it is restored or
-- purged. A passkey sign-in to the account stores the digest of a single-use,
-- short-lived restore token here, with the passkey that signed in; the token
-- restores the account and signs in with that passkey.
--
-- A purge that fails is retried with a backoff, so accounts that keep failing
-- never hold back every other due account. next_attempt_at is NULL until the
-- first failure: the purge is due at purge_after until then. last_error is the
-- error's message chain, never a value from the account.
CREATE TABLE account_deletions (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    purge_after TIMESTAMPTZ NOT NULL,
    restore_token_digest BYTEA UNIQUE CHECK (octet_length(restore_token_digest) = 32),
    restore_token_expires_at TIMESTAMPTZ,
    restore_passkey_id UUID,
    attempts INT NOT NULL DEFAULT 0,
    last_error TEXT,
    next_attempt_at TIMESTAMPTZ,
    CHECK ((restore_token_digest IS NULL) = (restore_token_expires_at IS NULL)),
    CHECK ((restore_token_digest IS NULL) = (restore_passkey_id IS NULL)),
    CHECK ((attempts = 0) = (last_error IS NULL)),
    CHECK ((last_error IS NULL) = (next_attempt_at IS NULL))
);
-- The purge job reads due accounts in the order they became due. greatest()
-- ignores a NULL next_attempt_at.
CREATE INDEX account_deletions_due_idx
    ON account_deletions ((greatest(next_attempt_at, purge_after)));

-- An account that stops being 'deleting' is no longer scheduled, however it left:
-- restored, enabled or disabled. Otherwise the purge would skip it forever, and
-- it could never be scheduled again.
CREATE FUNCTION end_account_deletion() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    DELETE FROM account_deletions WHERE user_id = NEW.id;
    RETURN NULL;
END $$;
CREATE TRIGGER users_end_account_deletion
    AFTER UPDATE OF status ON users
    FOR EACH ROW WHEN (OLD.status = 'deleting' AND NEW.status <> 'deleting')
    EXECUTE FUNCTION end_account_deletion();

----------------------------------------------------------------------------
-- Sessions and devices
----------------------------------------------------------------------------

-- passkey_confirmed_at is when the session last proved possession of a passkey;
-- each protected action states the maximum age it accepts. A session is
-- phishing-resistant exactly when it has one, and then lists passkeys among its
-- methods. passkey_id is the passkey that last proved it: the one that signed it
-- in, or confirmed it since; NULL for a social sign-in never confirmed. Undoing
-- an account recovery needs a session of a passkey enrolled before it, so NULL
-- never counts. It is no foreign key: it is only compared with passkeys, and a
-- purge may delete either row first.
--
-- user_agent_family is the coarse browser and platform, `browser/platform`, e.g.
-- `firefox/linux`. The IPs, for the account's list of signed-in devices, live
-- only as long as the session row.
CREATE TABLE sessions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    authentication_methods JSONB NOT NULL,
    authenticated_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    last_seen_at TIMESTAMPTZ NOT NULL,
    user_agent_family TEXT,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    passkey_confirmed_at TIMESTAMPTZ,
    created_ip INET,
    last_seen_ip INET,
    passkey_id UUID,
    CHECK (expires_at > authenticated_at)
);
CREATE INDEX sessions_active_user_idx ON sessions (user_id, expires_at) WHERE revoked_at IS NULL;
CREATE INDEX sessions_user_idx ON sessions (user_id);

-- Browser cookies are stored only as digests.
CREATE TABLE browser_session_secrets (
    session_id UUID PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    secret_digest BYTEA NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The browsers each account has signed in from, so a sign-in from any other
-- one alerts its owner. A browser is known by the digest of its device cookie,
-- a random secret it keeps for 400 days, apart from its session cookie, so
-- signing out does not forget it. Maintenance deletes a device unused for 400
-- days, and deleting the account deletes its devices.
CREATE TABLE known_devices (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_digest BYTEA NOT NULL CHECK (octet_length(device_digest) = 32),
    user_agent_family TEXT,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, device_digest)
);
-- Maintenance finds the devices unused for 400 days.
CREATE INDEX known_devices_last_seen_idx ON known_devices (last_seen_at);

----------------------------------------------------------------------------
-- Passkeys
----------------------------------------------------------------------------

CREATE TABLE passkey_credentials (
    id UUID PRIMARY KEY,
    credential_id BYTEA NOT NULL UNIQUE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential JSONB NOT NULL,
    friendly_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX passkey_credentials_active_user_idx ON passkey_credentials (user_id, created_at) WHERE revoked_at IS NULL;
CREATE INDEX passkey_credentials_user_idx ON passkey_credentials (user_id);
-- Cleanup deletes passkeys revoked 90 days ago.
CREATE INDEX passkey_credentials_revoked_idx ON passkey_credentials (revoked_at)
    WHERE revoked_at IS NOT NULL;

-- An account keeps at most 100 active passkeys, so the recovery event listing
-- them stays well under the event payload bound below. Nobody needs more.
-- Concurrent registrations can pass it by a few, which the bound still fits.
CREATE FUNCTION limit_passkeys() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF (
        SELECT count(*) FROM passkey_credentials
        WHERE user_id = NEW.user_id AND revoked_at IS NULL
    ) < 100 THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'account % already has 100 active passkeys', NEW.user_id
        USING ERRCODE = 'check_violation';
END $$;
CREATE TRIGGER passkey_credentials_limit
    BEFORE INSERT ON passkey_credentials
    FOR EACH ROW EXECUTE FUNCTION limit_passkeys();

-- Ceremony state includes the challenge and is stored server-side until expiry.
-- It is never accepted twice and must never appear in logs. A
-- 'related_authentication' is a passkey sign-in on a related origin, bound to
-- its origin and the pushed authorization request it completes, rather than to a
-- browser cookie.
CREATE TABLE webauthn_ceremonies (
    id UUID PRIMARY KEY,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    session_id UUID REFERENCES sessions(id) ON DELETE CASCADE,
    ceremony_kind TEXT NOT NULL CHECK (ceremony_kind IN (
        'registration', 'authentication', 'enrollment', 'purpose_authentication',
        'related_authentication'
    )),
    challenge_digest BYTEA NOT NULL,
    state JSONB NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX webauthn_ceremonies_expiry_idx ON webauthn_ceremonies (expires_at) WHERE consumed_at IS NULL;

----------------------------------------------------------------------------
-- Account recoveries
----------------------------------------------------------------------------

-- Recoveries an operator started for an account whose owner lost their
-- passkeys. Each sends its owner a link that enrolls one passkey, once, within a
-- day. The link's token is derived from the recovery's ID, so only its digest is
-- stored. For 15 days after the link is used, the recovery is in probation: its
-- passkey signs in but never confirms a session, and a passkey from before it
-- can undo it.
CREATE TABLE account_recoveries (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    secret_digest BYTEA NOT NULL UNIQUE,
    started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    -- When its link enrolled a passkey, which one, and when its probation ends.
    -- `settles_at` is computed by auth from its probation length, so SQL never
    -- repeats the constant.
    completed_at TIMESTAMPTZ,
    passkey_id UUID,
    settles_at TIMESTAMPTZ,
    -- When a newer recovery, the account's disabling or its owner's undo cancelled
    -- it before its link was used.
    cancelled_at TIMESTAMPTZ,
    -- When it was undone after its link was used, by its owner or by support.
    undone_at TIMESTAMPTZ,
    CHECK ((completed_at IS NULL) = (passkey_id IS NULL)),
    CHECK ((completed_at IS NULL) = (settles_at IS NULL)),
    CHECK (cancelled_at IS NULL OR (completed_at IS NULL AND undone_at IS NULL)),
    CHECK (undone_at IS NULL OR completed_at IS NOT NULL)
);
CREATE INDEX account_recoveries_user_idx ON account_recoveries (user_id, started_at DESC);
CREATE UNIQUE INDEX account_recoveries_passkey_idx ON account_recoveries (passkey_id);
-- At most one recovery per account whose link was not used, cancelled or undone.
-- An expired one counts until the next start cancels it.
CREATE UNIQUE INDEX account_recoveries_one_open_idx ON account_recoveries (user_id)
    WHERE completed_at IS NULL AND cancelled_at IS NULL AND undone_at IS NULL;

-- The account's recovery that can still be undone, if any: its link still works,
-- or it is in probation (its rows with `completed_at` set). The one SQL copy of
-- auth's `RecoveryState`; only the trigger functions below call it, as their
-- owner.
CREATE FUNCTION undoable_recovery(account UUID) RETURNS SETOF account_recoveries
LANGUAGE sql VOLATILE SET search_path = pg_catalog, public, pg_temp AS $$
    SELECT * FROM public.account_recoveries
    WHERE user_id = account
    AND undone_at IS NULL AND cancelled_at IS NULL
    AND (settles_at > clock_timestamp()
        OR (completed_at IS NULL AND expires_at > clock_timestamp()))
$$;
REVOKE EXECUTE ON FUNCTION undoable_recovery(UUID) FROM PUBLIC;

-- The triggers below hold the recovery's rules for every role that writes their
-- tables. They run as their owner, so a role not granted `account_recoveries`
-- cannot get past them, and no one else may execute them.

-- A probation passkey never confirms a session. A confirmation that names no
-- passkey fails closed while the account is in probation.
CREATE FUNCTION sessions_no_probation_confirmation() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF NEW.passkey_confirmed_at IS NOT NULL AND EXISTS (
        SELECT 1 FROM public.undoable_recovery(NEW.user_id)
        WHERE completed_at IS NOT NULL
        AND (NEW.passkey_id IS NULL OR passkey_id = NEW.passkey_id)
    ) THEN
        NEW.passkey_confirmed_at := NULL;
    END IF;
    RETURN NEW;
END $$;
REVOKE EXECUTE ON FUNCTION sessions_no_probation_confirmation() FROM PUBLIC;
CREATE TRIGGER sessions_no_probation_confirmation
    BEFORE INSERT OR UPDATE OF passkey_confirmed_at, passkey_id ON sessions
    FOR EACH ROW EXECUTE FUNCTION sessions_no_probation_confirmation();

-- No account is made an administrator while a recovery of it can be undone:
-- the rights would go live for whoever holds its passkey once it settles.
CREATE FUNCTION administrators_no_grant_mid_recovery() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM public.undoable_recovery(NEW.user_id)) THEN
        RAISE EXCEPTION 'account % has a recovery that can still be undone', NEW.user_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE EXECUTE ON FUNCTION administrators_no_grant_mid_recovery() FROM PUBLIC;
CREATE TRIGGER administrators_no_grant_mid_recovery
    BEFORE INSERT ON administrators
    FOR EACH ROW EXECUTE FUNCTION administrators_no_grant_mid_recovery();

-- While a recovery can be undone, no passkey but its own is revoked, by anyone,
-- so nothing takes away the passkeys that can undo it.
CREATE FUNCTION passkeys_kept_mid_recovery() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM public.undoable_recovery(OLD.user_id)
        WHERE passkey_id IS DISTINCT FROM OLD.id
    ) THEN
        RAISE EXCEPTION 'account % has a recovery that can still be undone', OLD.user_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE EXECUTE ON FUNCTION passkeys_kept_mid_recovery() FROM PUBLIC;
CREATE TRIGGER passkeys_kept_mid_recovery
    BEFORE UPDATE OF revoked_at ON passkey_credentials
    FOR EACH ROW WHEN (OLD.revoked_at IS NULL AND NEW.revoked_at IS NOT NULL)
    EXECUTE FUNCTION passkeys_kept_mid_recovery();

----------------------------------------------------------------------------
-- OIDC clients
----------------------------------------------------------------------------

-- Operators manage clients with `auth-service client`, and accounts manage the
-- ones they register through the account API. owner_id is NULL for operators'
-- clients, which the account API never shows. created_via records which path
-- registered the client; 'registration' is reserved for dynamic client
-- registration (#44).
--
-- Deletion is a tombstone: the row stays, so its ID, which tokens carry as `aud`,
-- is never reused, and cleanup never deletes it. A tombstone is always disabled,
-- so no path can bring it back.
--
-- A suspension is an operator's block: the client is disabled, and its owner
-- cannot enable it until an operator lifts the suspension.
--
-- Purging an account keeps its clients' tombstones, without their owner. Its live
-- clients must be deleted first: the owner check refuses to turn them into clients
-- no account manages.
--
-- last_used_at moves at most once an hour, when a token is issued to the client.
-- backchannel_logout_uri is NULL for clients that are not told when a session
-- ends (OIDC Back-Channel Logout 1.0).
CREATE TABLE oidc_clients (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    trusted BOOLEAN NOT NULL DEFAULT FALSE,
    allowed_scopes JSONB NOT NULL,
    allowed_audiences JSONB NOT NULL,
    allow_refresh BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    client_type TEXT NOT NULL DEFAULT 'public' CHECK (client_type IN ('public', 'confidential')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Clients that may start an authorization only from a pushed request.
    require_pushed_authorization_requests BOOLEAN NOT NULL DEFAULT FALSE,
    owner_id UUID REFERENCES users(id) ON DELETE SET NULL,
    created_via TEXT NOT NULL DEFAULT 'cli' CHECK (created_via IN ('cli', 'account', 'registration')),
    deleted_at TIMESTAMPTZ,
    suspended_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    backchannel_logout_uri TEXT,
    CONSTRAINT oidc_clients_tombstone_disabled CHECK (deleted_at IS NULL OR NOT enabled),
    CONSTRAINT oidc_clients_suspended_disabled CHECK (suspended_at IS NULL OR NOT enabled),
    CONSTRAINT oidc_clients_owned_until_deleted
        CHECK (created_via <> 'account' OR owner_id IS NOT NULL OR deleted_at IS NOT NULL)
);
-- Counts an owner's live clients for the quota, and the clients, deleted or not,
-- they registered recently for the daily registration limit.
CREATE INDEX oidc_clients_owner_idx ON oidc_clients (owner_id, created_at);

-- A client may hold several secrets at once: rotating one caps the expiry of the
-- previous secrets, so the client keeps working while it redeploys. Only SHA-256
-- digests are stored; auth generates every secret with 256 random bits.
CREATE TABLE oidc_client_secrets (
    id UUID PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES oidc_clients(id) ON DELETE CASCADE,
    secret_digest BYTEA NOT NULL CHECK (octet_length(secret_digest) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- NULL until a rotation sets it on the previous secrets.
    expires_at TIMESTAMPTZ
);
CREATE INDEX oidc_client_secrets_client_idx ON oidc_client_secrets (client_id);

-- origin is the redirect URI's origin, as browsers send it in an Origin header,
-- which auth computes on write. A public client's authorization redirect origins
-- may call the token and UserInfo endpoints, which look them up per request.
CREATE TABLE oidc_redirect_uris (
    client_id TEXT NOT NULL REFERENCES oidc_clients(id) ON DELETE CASCADE,
    uri TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('authorization', 'post_logout')),
    origin TEXT NOT NULL,
    PRIMARY KEY (client_id, uri, kind)
);
CREATE INDEX oidc_redirect_uris_origin_idx ON oidc_redirect_uris (origin)
    WHERE kind = 'authorization';

-- A deleted client keeps only its ID, settings that identify nobody, and its
-- lifecycle dates: its name, redirect URIs and back-channel logout URI may name a
-- person or their domain. Its name becomes empty.
CREATE FUNCTION scrub_deleted_client() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    NEW.display_name := '';
    NEW.backchannel_logout_uri := NULL;
    DELETE FROM oidc_redirect_uris WHERE client_id = NEW.id;
    RETURN NEW;
END $$;
CREATE TRIGGER oidc_clients_scrub_deleted
    BEFORE UPDATE OF deleted_at ON oidc_clients
    FOR EACH ROW WHEN (OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL)
    EXECUTE FUNCTION scrub_deleted_client();

-- Origins whose pages may sign users in with their auth passkeys (WebAuthn
-- related origins). `/.well-known/webauthn` lists them, so browsers let those
-- pages use auth's RP ID. Only operators approve them: browsers honour few
-- distinct domains, and the client must be trusted and have a redirect URI on
-- the origin. The origin is written as browsers send it, e.g. https://app.example.
CREATE TABLE client_webauthn_origins (
    origin TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES oidc_clients(id) ON DELETE CASCADE,
    approved_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX client_webauthn_origins_client_idx ON client_webauthn_origins (client_id);

-- A deleted client's origins go with it: clients are tombstoned, so the foreign
-- key never cascades.
CREATE FUNCTION remove_deleted_client_origins() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    DELETE FROM client_webauthn_origins WHERE client_id = NEW.id;
    RETURN NULL;
END $$;
CREATE TRIGGER oidc_clients_remove_related_origins
    AFTER UPDATE OF deleted_at ON oidc_clients
    FOR EACH ROW WHEN (OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL)
    EXECUTE FUNCTION remove_deleted_client_origins();

-- Signing keys. Every replica reloads them each minute; one replica at a time
-- advances their rotation, under a transaction-level advisory lock that the
-- `keys` commands take too. A key is published as `next`, signs as `active`, stays
-- published as `retiring`, then verifies only `id_token_hint`s as `retired` until
-- it is deleted. Its PKCS#8 private key is sealed with AES-256-GCM under the
-- key-encryption key from configuration, with `id || key_type` as associated
-- data, and erased once it retires.
CREATE TABLE signing_keys (
    id TEXT PRIMARY KEY CHECK (length(id) BETWEEN 1 AND 128),
    key_type TEXT NOT NULL CHECK (key_type IN ('rsa')),
    state TEXT NOT NULL CHECK (state IN ('next', 'active', 'retiring', 'retired')),
    public_jwk JSONB NOT NULL,
    sealed_private_key BYTEA,
    created_at TIMESTAMPTZ NOT NULL,
    -- When it started signing, stopped signing, and left the key set.
    activated_at TIMESTAMPTZ,
    deactivated_at TIMESTAMPTZ,
    retired_at TIMESTAMPTZ,
    CHECK ((state = 'retired') = (sealed_private_key IS NULL)),
    CHECK ((state = 'next') = (activated_at IS NULL)),
    CHECK ((state IN ('next', 'active')) = (deactivated_at IS NULL)),
    CHECK ((state = 'retired') = (retired_at IS NOT NULL))
);
CREATE UNIQUE INDEX signing_keys_one_active ON signing_keys (key_type) WHERE state = 'active';
CREATE UNIQUE INDEX signing_keys_one_next ON signing_keys (key_type) WHERE state = 'next';

----------------------------------------------------------------------------
-- Authorization, consent and refresh tokens
----------------------------------------------------------------------------

-- Every secret digest records the fingerprint of the token digest key it was
-- made with (`digest_key`), so `auth-service keys digest-usage` can tell when no
-- stored digest needs an old key.

-- A validated authorization request, resumed across login and consent.
-- hinted_user_id keeps the requested subject across that resumption. client_state
-- is NULL when the client sent no state (RFC 6749 §4.1.1), and pkce_challenge
-- when a confidential client left PKCE out (RFC 9700). target_acr is the class
-- the client asked for with `acr_values`, if it named a known one. interaction is
-- the request's `prompt`: `none` excludes `login` and `consent`.
--
-- A pushed authorization request (RFC 9126) is stored by its client through the
-- back channel, then claimed once by the browser with the returned request_uri.
-- It is not a pending authorization until it is claimed: claiming replaces its
-- secret, so the request_uri itself never continues one.
CREATE TABLE authorization_requests (
    id UUID PRIMARY KEY,
    secret_digest BYTEA NOT NULL UNIQUE,
    client_id TEXT NOT NULL REFERENCES oidc_clients(id),
    redirect_uri TEXT NOT NULL,
    client_state TEXT,
    nonce TEXT,
    pkce_challenge TEXT,
    scopes JSONB NOT NULL,
    audience TEXT NOT NULL,
    max_age BIGINT,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    hinted_user_id UUID,
    target_acr TEXT,
    pushed BOOLEAN NOT NULL DEFAULT FALSE,
    claimed_at TIMESTAMPTZ,
    digest_key TEXT NOT NULL,
    interaction TEXT NOT NULL
        CHECK (interaction IN ('as_needed', 'login', 'consent', 'login_consent', 'none'))
);
CREATE INDEX authorization_requests_active_idx ON authorization_requests (expires_at) WHERE consumed_at IS NULL;

-- authentication is `SessionAuthentication` as JSON: `acr`, `auth_time` and
-- `amr`, from when the code was issued, so later re-authentication of the
-- session never changes what the grant reports (OIDC Core 12.2).
CREATE TABLE authorization_codes (
    id UUID PRIMARY KEY,
    secret_digest BYTEA NOT NULL UNIQUE,
    client_id TEXT NOT NULL REFERENCES oidc_clients(id),
    user_id UUID NOT NULL REFERENCES users(id),
    session_id UUID NOT NULL REFERENCES sessions(id),
    redirect_uri TEXT NOT NULL,
    pkce_challenge TEXT,
    nonce TEXT,
    scopes JSONB NOT NULL,
    audience TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    authentication JSONB NOT NULL,
    digest_key TEXT NOT NULL
);
CREATE INDEX authorization_codes_active_idx ON authorization_codes (expires_at) WHERE consumed_at IS NULL;
-- Lets cleanup find redeemed codes past their grace period without a full scan.
CREATE INDEX authorization_codes_consumed_idx ON authorization_codes (consumed_at) WHERE consumed_at IS NOT NULL;

CREATE TABLE oidc_consents (
    user_id UUID NOT NULL REFERENCES users(id),
    client_id TEXT NOT NULL REFERENCES oidc_clients(id) ON DELETE CASCADE,
    audience TEXT NOT NULL,
    scopes JSONB NOT NULL,
    PRIMARY KEY (user_id, client_id, audience)
);
-- Deleting a client deletes its consents.
CREATE INDEX oidc_consents_client_idx ON oidc_consents (client_id);

-- Which clients received tokens from which session, so that when the session
-- ends auth posts a logout token to the back-channel logout URI of each.
CREATE TABLE oidc_session_clients (
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL REFERENCES oidc_clients(id) ON DELETE CASCADE,
    first_issued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (session_id, client_id)
);

-- Every redeemed code records its client. An exchange that fails rolls back the
-- redemption, so only clients that received tokens are recorded. It runs as its
-- owner, so it records the client for every role that redeems a code. No one
-- else may execute it: on a temporary table of its own, with a type whose cast to
-- uuid it wrote, the runtime role would run that cast as the owner. A trigger
-- already attached still fires.
CREATE FUNCTION oidc_session_clients_record_redeemed_code() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    INSERT INTO oidc_session_clients (session_id, client_id)
    VALUES (NEW.session_id, NEW.client_id)
    ON CONFLICT DO NOTHING;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION oidc_session_clients_record_redeemed_code() FROM PUBLIC;
CREATE TRIGGER authorization_codes_record_session_client
AFTER UPDATE OF consumed_at ON authorization_codes
FOR EACH ROW WHEN (OLD.consumed_at IS NULL AND NEW.consumed_at IS NOT NULL)
EXECUTE FUNCTION oidc_session_clients_record_redeemed_code();

-- A family groups every rotation of one grant; revoking it ends the grant. It
-- keeps the authentication of the code it was redeemed from, and links to that
-- code, so redeeming the code again revokes the family (RFC 6749 section 4.1.2).
-- Cleanup keeps a redeemed code while a family it produced can still refresh;
-- purging it afterwards clears the link rather than failing on it.
CREATE TABLE refresh_token_families (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    client_id TEXT NOT NULL REFERENCES oidc_clients(id),
    session_id UUID NOT NULL REFERENCES sessions(id),
    audience TEXT NOT NULL,
    scopes JSONB NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    authorization_code_id UUID REFERENCES authorization_codes(id) ON DELETE SET NULL,
    authentication JSONB NOT NULL
);
CREATE INDEX refresh_families_session_idx ON refresh_token_families (session_id);
CREATE INDEX refresh_families_active_user_idx ON refresh_token_families (user_id) WHERE revoked_at IS NULL;
CREATE INDEX refresh_families_user_idx ON refresh_token_families (user_id);
-- A client's deletion locks, then revokes, its live families.
CREATE INDEX refresh_families_active_client_idx ON refresh_token_families (client_id)
    WHERE revoked_at IS NULL;
-- Serves replay revocation, cleanup's live-family check and the ON DELETE SET
-- NULL lookup when codes are purged.
CREATE INDEX refresh_families_authorization_code_idx ON refresh_token_families (authorization_code_id);

-- A rotated refresh token names the token it replaced, so a public client that
-- presents that token again moments later, from a second tab or as a retry, is
-- answered with this successor instead of having its family revoked as a
-- replay. A token has at most one successor.
CREATE TABLE refresh_tokens (
    id UUID PRIMARY KEY,
    family_id UUID NOT NULL REFERENCES refresh_token_families(id) ON DELETE CASCADE,
    secret_digest BYTEA NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    consumed_at TIMESTAMPTZ,
    digest_key TEXT NOT NULL,
    predecessor_id UUID UNIQUE REFERENCES refresh_tokens(id) ON DELETE SET NULL
);
CREATE INDEX refresh_tokens_family_idx ON refresh_tokens (family_id);

----------------------------------------------------------------------------
-- Social login federation
----------------------------------------------------------------------------

CREATE TABLE external_login_identities (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider_name TEXT NOT NULL CHECK (provider_name IN ('github', 'google')),
    issuer TEXT NOT NULL,
    subject TEXT NOT NULL CHECK (length(subject) BETWEEN 1 AND 255),
    linked_email TEXT,
    linked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ
);
-- Unlinking releases a provider subject, but keeps its audit history.
CREATE UNIQUE INDEX external_identity_active_subject ON external_login_identities (issuer, subject) WHERE revoked_at IS NULL;
CREATE UNIQUE INDEX external_identity_active_provider ON external_login_identities (user_id, provider_name) WHERE revoked_at IS NULL;
CREATE INDEX external_login_identities_user_idx ON external_login_identities (user_id);
-- Cleanup deletes identities unlinked 90 days ago.
CREATE INDEX external_login_identities_revoked_idx ON external_login_identities (revoked_at)
    WHERE revoked_at IS NOT NULL;

-- An in-flight provider redirect, bound to the initiating browser.
CREATE TABLE federation_transactions (
    id UUID PRIMARY KEY,
    provider_name TEXT NOT NULL CHECK (provider_name IN ('github', 'google')),
    state_digest BYTEA NOT NULL UNIQUE CHECK (octet_length(state_digest)=32),
    browser_digest BYTEA NOT NULL CHECK (octet_length(browser_digest)=32),
    nonce TEXT NOT NULL,
    pkce_verifier TEXT NOT NULL,
    return_to TEXT NOT NULL,
    link_user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    link_session_id UUID REFERENCES sessions(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((link_user_id IS NULL) = (link_session_id IS NULL))
);
CREATE INDEX federation_transactions_expiry_idx ON federation_transactions (expires_at);

-- No user or normal session exists while social signup is pending. The browser
-- must finish its first passkey before this identity can be committed.
-- picture_url is the profile picture the provider reported, which the signup
-- imports once the account exists. email_authoritative says whether the
-- provider is authoritative for the address, so it is imported verified.
CREATE TABLE pending_federation (
    id UUID PRIMARY KEY,
    browser_digest BYTEA NOT NULL UNIQUE CHECK (octet_length(browser_digest)=32),
    provider_name TEXT NOT NULL CHECK (provider_name IN ('github', 'google')),
    issuer TEXT NOT NULL,
    subject TEXT NOT NULL CHECK (length(subject) BETWEEN 1 AND 255),
    email TEXT,
    display_name TEXT NOT NULL,
    return_to TEXT NOT NULL,
    link_user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    link_session_id UUID REFERENCES sessions(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    picture_url TEXT CHECK (char_length(picture_url) <= 2048),
    email_authoritative BOOLEAN NOT NULL DEFAULT false,
    CHECK (link_session_id IS NULL OR link_user_id IS NOT NULL),
    CHECK (email IS NOT NULL OR NOT email_authoritative)
);
CREATE INDEX pending_federation_expiry_idx ON pending_federation (expires_at);

----------------------------------------------------------------------------
-- Invitations and rate limits
----------------------------------------------------------------------------

-- Administrators list and revoke invitations by ID, without knowing their
-- secret; a revoked invitation can no longer be redeemed. An invitation may be
-- mailed to the person it invites: it remembers their normalized address, which
-- their account starts with, verified, when they sign up with it. Its link's
-- token is derived from its ID, so the email sender rebuilds the link from the
-- stored digest, as it does a recovery's. Cleanup deletes the address with its
-- invitation, once it expires or a day after it is redeemed.
CREATE TABLE invitations (
    id UUID PRIMARY KEY,
    secret_digest BYTEA NOT NULL UNIQUE CHECK (octet_length(secret_digest) = 32),
    expires_at TIMESTAMPTZ NOT NULL,
    redeemed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ,
    email TEXT
);

-- Fixed-window request counters shared by every service process.
CREATE TABLE request_limits (
    bucket BYTEA PRIMARY KEY,
    requests INTEGER NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);

----------------------------------------------------------------------------
-- Domain events and the audit chain
----------------------------------------------------------------------------

-- Typed domain events. domain_events holds only non-identifying content: the
-- event kind and payload, its actor, and the subject, client and session it
-- concerns. Anything that identifies a person outside auth (client IP, user agent
-- family, and every Personal value of the payload) lives in event_personal_data,
-- which retention and account purges delete without touching the event.
--
-- subject_user_id has no foreign key: history outlives the account it describes.
--
-- The history is tamper-evident: a hash chain over sealed events, and signed
-- checkpoints of its head. See docs/dev/security/audit-history.md. The sealer, one
-- replica at a time, gives each event its chain position and hash after its
-- transaction and every older one have finished; events are inserted unsealed.
-- Each sealed event records the format of the canonical event its hash covers,
-- which the canonical event names, so a later format can add fields without
-- breaking the hashes of events sealed before it. Only known formats are allowed,
-- so a forged one is reported where it is rather than taken for a newer
-- release's; the release that adds a format widens the check.
CREATE TABLE domain_events (
    id UUID PRIMARY KEY,
    txid XID8 NOT NULL DEFAULT pg_current_xact_id(),
    sequence BIGINT GENERATED ALWAYS AS IDENTITY UNIQUE,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    kind TEXT NOT NULL,
    version SMALLINT NOT NULL,
    payload JSONB NOT NULL,
    actor JSONB NOT NULL,
    subject_user_id UUID,
    client_id TEXT,
    session_id UUID,
    -- The request's trace ID, which links the event to its telemetry.
    trace_id TEXT,
    chain_position BIGINT UNIQUE CHECK (chain_position > 0),
    -- SHA-256(previous chain_hash || SHA-256(canonical event)).
    chain_hash BYTEA CHECK (octet_length(chain_hash) = 32),
    chain_format SMALLINT NOT NULL DEFAULT 1 CHECK (chain_format IN (1)),
    CHECK ((chain_position IS NULL) = (chain_hash IS NULL))
);
-- Consumers read in (txid, sequence) order; see EventRepository::read_after.
CREATE INDEX domain_events_cursor_idx ON domain_events (txid, sequence);
-- The sealer reads unsealed events in that order too.
CREATE INDEX domain_events_unsealed_idx ON domain_events (txid, sequence)
    WHERE chain_position IS NULL;
-- Histories list events in the order they are sealed in, (txid, sequence), which
-- verification checks against the chain, rather than by sequence alone, which the
-- chain does not cover.
CREATE INDEX domain_events_subject_order_idx
    ON domain_events (subject_user_id, txid DESC, sequence DESC);
-- A purge erases the personal data of the events the account acted in, found by
-- the actor's user ID. The index is not partial: PostgreSQL keeps no statistics
-- on a partial index's expression, and without them it estimates so many matches
-- that it scans every event's personal data instead.
CREATE INDEX domain_events_actor_user_idx ON domain_events ((actor ->> 'user_id'));
-- Purging an owner erases its clients' related origins from the events about them.
CREATE INDEX domain_events_client_idx ON domain_events (client_id)
    WHERE client_id IS NOT NULL;
-- The overviews count events of a kind over the last hour and week, across
-- every account: the dashboard's failed and successful sign-ins and code
-- exchanges, against what an hour usually brings.
CREATE INDEX domain_events_kind_time_idx ON domain_events (kind, occurred_at);

-- Auth's runtime role writes history only the way auth does (#250), so a
-- compromised auth can neither forge history nor plant rows that freeze it. With
-- the column grants of runtime_role.sql, it cannot set an event's transaction,
-- time, sequence or chain columns when inserting it. The guards below refuse the
-- rest, for every role: an event auth could not read back, personal data
-- attached to another transaction's event, a seal that skips a position, and a
-- checkpoint for anything but a sealed event's hash, or dated in the future. A
-- huge position, an unreadable event or a future checkpoint would otherwise stop
-- sealing or signing until the schema owner repaired it. SQL cannot check a hash
-- or a checkpoint's signature; the sealer and `audit verify` do, and a seal with
-- a wrong one still stops signing until the owner repairs the chain.
--
-- An event the sealer could not read would fail every sealing pass, which loads
-- up to 100 events at once. serde_json refuses numbers beyond a double's range
-- and nesting past 128 levels, and a huge event would exhaust the sealer's memory
-- or time: parsed, JSON can take about 100 times its text. So an event's payload
-- stays under 8 KiB, its actor under 1 KiB and its text columns under 256 bytes,
-- about 95 MB for a batch at worst. Real events are a few hundred bytes, the
-- largest a recovery listing an account's passkeys, which the passkey limit
-- keeps around 4 KiB. Sealing an event updates it and checks it again.
CREATE FUNCTION auth_reads_json(value JSONB, bytes INT) RETURNS BOOLEAN
LANGUAGE sql IMMUTABLE SET search_path = pg_catalog, pg_temp AS $$
    SELECT octet_length(value::text) <= bytes AND NOT (
        value @? '$.** ? (@.type() == "number" && (@ > 1e308 || @ < -1e308))'
        OR value @? '$.**{64 to last}'
    )
$$;
ALTER TABLE domain_events ADD CONSTRAINT domain_events_readable CHECK (
    auth_reads_json(payload, 8192) AND auth_reads_json(actor, 1024)
    AND octet_length(kind) <= 256 AND octet_length(client_id) <= 256
    AND octet_length(trace_id) <= 256
);

-- details maps the JSON pointer of each Personal payload value to that value.
-- user_agent_family holds the browser of the event's user agent, and platform its
-- platform. reason is why an administrator or operator changed someone's
-- account, as they wrote it: free text may name people.
CREATE TABLE event_personal_data (
    event_id UUID PRIMARY KEY REFERENCES domain_events(id) ON DELETE CASCADE,
    client_ip INET,
    user_agent_family TEXT,
    details JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    platform TEXT,
    reason TEXT
);
-- Lets cleanup find personal data past its retention without a full scan.
CREATE INDEX event_personal_data_created_idx ON event_personal_data (created_at);

-- Personal data is recorded with its event, never attached to an older one: with
-- the deletes retention needs, that would let it be rewritten.
CREATE FUNCTION guard_personal_data_insert() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (
        SELECT FROM domain_events WHERE id = NEW.event_id AND txid = pg_current_xact_id()
    ) THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'personal data of event % can only be recorded with the event', NEW.event_id
        USING ERRCODE = 'integrity_constraint_violation';
END $$;
CREATE TRIGGER event_personal_data_guard_insert
    BEFORE INSERT ON event_personal_data
    FOR EACH ROW EXECUTE FUNCTION guard_personal_data_insert();

-- An event keeps personal data only while every account it names, as subject or
-- actor, exists: a request still in flight when its account is purged must not
-- store it again. Each named account is key-share locked, which only deleting
-- the account conflicts with: either the purge's delete waits, then erases this
-- row, or this insert waits, finds the account gone and is skipped.
CREATE FUNCTION skip_personal_data_of_purged_accounts() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF EXISTS (
        SELECT FROM domain_events e
        CROSS JOIN LATERAL (
            VALUES (e.subject_user_id), ((e.actor ->> 'user_id')::uuid)
        ) AS named(id)
        WHERE e.id = NEW.event_id
        AND named.id IS NOT NULL
        AND NOT EXISTS (SELECT FROM users WHERE id = named.id FOR KEY SHARE)
    ) THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER event_personal_data_skip_purged_accounts
    BEFORE INSERT ON event_personal_data
    FOR EACH ROW EXECUTE FUNCTION skip_personal_data_of_purged_accounts();

-- Personal data is erased, never rewritten. Purging an app's owner erases the
-- app's related origins from other people's events by removing them from
-- `details`, the only column the runtime role may update; any other update fails.
CREATE FUNCTION guard_personal_data_erasure() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF NOT EXISTS (
        SELECT FROM jsonb_each(NEW.details) kept
        WHERE OLD.details -> kept.key IS DISTINCT FROM kept.value
    ) THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'personal data of event % can only be erased', OLD.event_id
        USING ERRCODE = 'integrity_constraint_violation';
END $$;
CREATE TRIGGER event_personal_data_guard_erasure
    BEFORE UPDATE ON event_personal_data
    FOR EACH ROW EXECUTE FUNCTION guard_personal_data_erasure();

-- Signed checkpoints of the chain head, in the C2SP signed-note format. Never
-- pruned: pruned events stay covered by the checkpoints that signed them.
CREATE TABLE audit_checkpoints (
    position BIGINT PRIMARY KEY CHECK (position > 0),
    head BYTEA NOT NULL CHECK (octet_length(head) = 32),
    created_at TIMESTAMPTZ NOT NULL,
    note TEXT NOT NULL
);

-- The checkpoint the chain was last pruned through, if it ever was. The first
-- remaining event chains from its head instead of from 32 zero bytes.
CREATE TABLE audit_anchor (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    position BIGINT NOT NULL REFERENCES audit_checkpoints (position)
);

-- Guards sealed history: a sealed event is never updated, and only pruning, which
-- sets sourceorigin.audit_prune for its transaction, deletes one. Checkpoints are never
-- updated or deleted, and neither table is truncated. The runtime role cannot
-- drop these triggers, and may update only unsealed events' chain columns (see
-- runtime_role.sql); the schema owner can, and the chain is what detects that.
CREATE FUNCTION guard_sealed_events() RETURNS TRIGGER LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF OLD.chain_position IS NULL
        OR (TG_OP = 'DELETE' AND current_setting('sourceorigin.audit_prune', true) = 'on') THEN
        RETURN CASE TG_OP WHEN 'DELETE' THEN OLD ELSE NEW END;
    END IF;
    RAISE EXCEPTION 'sealed event at chain position % cannot be changed', OLD.chain_position
        USING ERRCODE = 'integrity_constraint_violation';
END;
$$;
CREATE TRIGGER domain_events_guard_sealed
    BEFORE UPDATE OR DELETE ON domain_events
    FOR EACH ROW EXECUTE FUNCTION guard_sealed_events();

CREATE FUNCTION refuse_audit_history_change() RETURNS TRIGGER LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    RAISE EXCEPTION '% on % would change sealed audit history', TG_OP, TG_TABLE_NAME
        USING ERRCODE = 'integrity_constraint_violation';
END;
$$;
CREATE TRIGGER audit_checkpoints_guard
    BEFORE UPDATE OR DELETE ON audit_checkpoints
    FOR EACH ROW EXECUTE FUNCTION refuse_audit_history_change();
CREATE TRIGGER domain_events_guard_truncate
    BEFORE TRUNCATE ON domain_events
    FOR EACH STATEMENT EXECUTE FUNCTION refuse_audit_history_change();
CREATE TRIGGER audit_checkpoints_guard_truncate
    BEFORE TRUNCATE ON audit_checkpoints
    FOR EACH STATEMENT EXECUTE FUNCTION refuse_audit_history_change();
CREATE TRIGGER audit_anchor_guard_truncate
    BEFORE TRUNCATE ON audit_anchor
    FOR EACH STATEMENT EXECUTE FUNCTION refuse_audit_history_change();

-- The anchor moves only forward, and only while pruning, so it cannot hide events
-- left at or before it, and concurrent prunes cannot move it back.
CREATE FUNCTION guard_audit_anchor() RETURNS TRIGGER LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF TG_OP <> 'DELETE' AND current_setting('sourceorigin.audit_prune', true) = 'on' THEN
        IF TG_OP = 'INSERT' THEN
            RETURN NEW;
        END IF;
        IF NEW.position > OLD.position THEN
            RETURN NEW;
        END IF;
    END IF;
    RAISE EXCEPTION 'the audit anchor only moves forward, by pruning'
        USING ERRCODE = 'integrity_constraint_violation';
END;
$$;
CREATE TRIGGER audit_anchor_guard
    BEFORE INSERT OR UPDATE OR DELETE ON audit_anchor
    FOR EACH ROW EXECUTE FUNCTION guard_audit_anchor();

-- Each sealed position follows a sealed one, or the anchor, or starts the chain,
-- so no seal leaves a gap or jumps ahead. It runs once the statement has sealed
-- its whole batch, whatever order the rows were updated in. A wrong hash is for
-- verification to find: SQL cannot compute one.
--
-- Sealing must read committed data, as the sealer does: an older snapshot could
-- still show events a concurrent prune deleted, and seal behind the anchor. Read
-- committed, the check sees every committed prune, and a prune still deleting the
-- position makes the seal wait for it on the unique index first.
CREATE FUNCTION guard_seal_order() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'events are sealed only in read committed transactions'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.chain_position - 1 = coalesce((SELECT position FROM audit_anchor), 0)
        OR EXISTS (SELECT FROM domain_events WHERE chain_position = NEW.chain_position - 1)
    THEN
        RETURN NULL;
    END IF;
    RAISE EXCEPTION 'event % cannot be sealed at position %: the position before is not sealed',
        NEW.id, NEW.chain_position
        USING ERRCODE = 'integrity_constraint_violation';
END $$;
CREATE TRIGGER domain_events_guard_seal_order
    AFTER UPDATE OF chain_position ON domain_events
    FOR EACH ROW WHEN (OLD.chain_position IS NULL AND NEW.chain_position IS NOT NULL)
    EXECUTE FUNCTION guard_seal_order();

-- A checkpoint is of a sealed event's chain hash, and dated at a finite time
-- not in the future: auth cannot read an infinite one back. A minute of leeway
-- covers auth's clock running ahead of the database's. Its note, which auth reads
-- and checks a signature line by line, stays under 1 KiB; auth's are about 300
-- bytes.
CREATE FUNCTION guard_new_checkpoint() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF isfinite(NEW.created_at) AND octet_length(NEW.note) <= 1024
        AND NEW.created_at <= clock_timestamp() + INTERVAL '1 minute' AND EXISTS (
        SELECT FROM domain_events WHERE chain_position = NEW.position AND chain_hash = NEW.head
    ) THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'checkpoint at position % is not of the sealed chain as of now',
        NEW.position
        USING ERRCODE = 'integrity_constraint_violation';
END $$;
CREATE TRIGGER audit_checkpoints_guard_insert
    BEFORE INSERT ON audit_checkpoints
    FOR EACH ROW EXECUTE FUNCTION guard_new_checkpoint();

----------------------------------------------------------------------------
-- Deliveries
----------------------------------------------------------------------------

-- The outbound delivery queue: messages for other systems, such as back-channel
-- logout tokens or emails. DatabaseTransaction::record creates them in the
-- transaction of the event that causes them, and workers send them with leases.
-- See docs/dev/architecture/delivery-queue.md.
--
-- event_id has no foreign key, like domain_events.subject_user_id: audit pruning
-- deletes events, and a delivery must never depend on rows that can change after
-- it is created. destination and recipient copy everything a send needs.
CREATE TABLE deliveries (
    -- Also the delivery's identity for receivers: a jti or Message-ID.
    id UUID PRIMARY KEY,
    -- The event that caused it, when there is one.
    event_id UUID,
    channel TEXT NOT NULL,
    destination TEXT NOT NULL,
    recipient JSONB NOT NULL,
    -- Never a plaintext secret.
    payload JSONB NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'succeeded', 'failed', 'cancelled')),
    attempts INT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Set by the worker sending it. Only the holder of claim_token records the
    -- outcome; cancelling clears it.
    claim_token UUID,
    leased_until TIMESTAMPTZ,
    -- A status code or an error kind, never a response body.
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- When an operator last retried it; the channel's maximum age counts from here.
    retried_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);
-- Workers poll for due deliveries.
CREATE INDEX deliveries_due_idx ON deliveries (next_attempt_at) WHERE status = 'pending';
-- Cleanup finds finished deliveries past their retention.
CREATE INDEX deliveries_finished_idx ON deliveries (finished_at) WHERE status <> 'pending';
-- A purge deletes the emails and announcements addressed to the account.
CREATE INDEX deliveries_email_recipient_idx ON deliveries (recipient)
    WHERE channel = 'email';
CREATE INDEX deliveries_announcement_recipient_idx ON deliveries (recipient)
    WHERE channel = 'announcement';
-- The dashboard counts an announcement's emails by status while the queue keeps
-- them: their payload is the announcement's ID.
CREATE INDEX deliveries_announcement_idx ON deliveries (payload)
    WHERE channel = 'announcement';

----------------------------------------------------------------------------
-- Notifications and announcements
----------------------------------------------------------------------------

-- What each account's owner is told about their account, in their settings
-- inbox. A notification is written in the transaction of the event it comes
-- from, if any, and holds no personal data: it renders from its payload alone,
-- so it outlives its event's personal data and the event itself, which audit
-- pruning may delete. `event_id` is no foreign key for that reason. Purging the
-- account deletes its notifications with it.
CREATE TABLE notifications (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    event_id UUID,
    payload JSONB NOT NULL CHECK (jsonb_typeof(payload -> 'kind') = 'string'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    read_at TIMESTAMPTZ,
    -- When the "sign me out everywhere" link of its email was used. The link
    -- works once, within 7 days of the notification.
    signed_out_at TIMESTAMPTZ
);
-- The inbox, newest first. Notifications written in one transaction share
-- `now()`, so pages continue after `(created_at, id)`.
CREATE INDEX notifications_inbox_idx ON notifications (user_id, created_at DESC, id DESC);
-- The unread count every settings page shows.
CREATE INDEX notifications_unread_idx ON notifications (user_id) WHERE read_at IS NULL;
-- Cleanup deletes notifications a year after they arrive.
CREATE INDEX notifications_created_idx ON notifications (created_at);
-- How many of an announcement's recipients read it, which its page shows.
CREATE INDEX notifications_announcement_idx ON notifications ((payload ->> 'announcement_id'))
    WHERE payload ->> 'kind' = 'announcement';

-- Whether each account receives a category of notifications by email. Only
-- categories whose email the owner may turn off have rows, and no row means
-- the category's default: everything about the account itself is always
-- emailed. Deleting an account deletes its rows.
CREATE TABLE notification_preferences (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    category TEXT NOT NULL CHECK (category IN ('announcements')),
    email BOOLEAN NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, category)
);

-- Announcements administrators send to accounts, such as a security incident or
-- a change to the privacy policy: emailed to each recipient's primary verified
-- address, and written to each recipient's inbox as a notification.
--
-- An announcement is a draft until it is sent, and only a draft changes. Its
-- text is written for everyone it goes to, so it is no one's personal data,
-- except that an announcement to chosen accounts may concern them: a purge
-- deletes it with its last recipient.
CREATE TABLE announcements (
    id UUID PRIMARY KEY,
    subject TEXT NOT NULL CHECK (subject <> ''),
    body TEXT NOT NULL CHECK (body <> ''),
    -- Whether it goes to every account, rather than to the chosen ones.
    everyone BOOLEAN NOT NULL,
    -- Counts the draft's changes. Sending names the revision that was checked,
    -- so a change made meanwhile is checked again before anything is sent.
    revision BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at TIMESTAMPTZ,
    -- An essential announcement, such as a security incident or a change to the
    -- privacy policy or terms, is emailed to every recipient whatever their
    -- preference. Its sender chooses so on the draft.
    essential BOOLEAN NOT NULL DEFAULT false
);
CREATE INDEX announcements_recent_idx ON announcements (created_at DESC, id DESC);

-- What was sent stays as it was sent: the audit history names the announcement,
-- not its text.
CREATE FUNCTION guard_sent_announcement() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    RAISE EXCEPTION 'announcement % was sent, so it can no longer change', OLD.id
        USING ERRCODE = 'integrity_constraint_violation';
END $$;
CREATE TRIGGER announcements_sent_unchanged
    BEFORE UPDATE ON announcements
    FOR EACH ROW WHEN (OLD.sent_at IS NOT NULL)
    EXECUTE FUNCTION guard_sent_announcement();

-- The accounts an announcement goes to: the chosen ones from the draft on, or
-- every account, added when it is sent. Accounts created later never get it.
-- Deleting an account deletes its rows.
CREATE TABLE announcement_recipients (
    announcement_id UUID NOT NULL REFERENCES announcements(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (announcement_id, user_id)
);
-- A purge finds the announcements an account received, and the cascade its rows.
CREATE INDEX announcement_recipients_user_idx ON announcement_recipients (user_id);

-- Whether `account` receives announcements that are not essential by email:
-- unless its owner turned them off.
CREATE FUNCTION announcements_emailed(account UUID) RETURNS BOOLEAN
LANGUAGE sql STABLE SET search_path = pg_catalog, public, pg_temp AS $$
    SELECT NOT EXISTS (
        SELECT FROM notification_preferences p
        WHERE p.user_id = account AND p.category = 'announcements' AND NOT p.email
    )
$$;

----------------------------------------------------------------------------
-- Alerts
----------------------------------------------------------------------------

-- What needs an administrator's attention, such as a broken audit chain: a
-- condition auth found, which concerns no account. An alert is current from when
-- it is raised until its condition clears, and open until then too, unless an
-- administrator acknowledges it first: auth then raises it again only once its
-- condition clears or changes. At most one alert of each kind is current. Like
-- an event's payload, it holds no personal data, and its events tell who
-- acknowledged it.
CREATE TABLE alerts (
    id UUID PRIMARY KEY,
    payload JSONB NOT NULL CHECK (jsonb_typeof(payload -> 'kind') = 'string'),
    -- The payload's version, as an event's: a release reads one it does not
    -- write as an alert it cannot show.
    version SMALLINT NOT NULL,
    raised_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- When auth last found its condition holding.
    seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    acknowledged_at TIMESTAMPTZ,
    cleared_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX alerts_current_idx ON alerts ((payload ->> 'kind')) WHERE cleared_at IS NULL;

-- What administrators post on an alert while they handle it: where its handling
-- stands. The first acknowledges the alert, which then needs no attention while
-- its condition holds as it says. The author is the administrator or operator, as
-- an event's actor. Each update records an `alert_updated` event, `event_id`,
-- whose reason holds its note: the note is personal data, deleted with the
-- event's after 90 days or when its author's account is purged. No foreign key
-- holds `event_id`, since pruning the audit chain deletes events, and the update
-- outlives its event. An update is stamped when it is written, after its alert's
-- row is locked, not when its transaction began: updates sort in the order they
-- were posted even when their transactions overlap.
CREATE TABLE alert_updates (
    id UUID PRIMARY KEY,
    alert_id UUID NOT NULL REFERENCES alerts(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('investigating', 'fixing', 'monitoring')),
    event_id UUID NOT NULL UNIQUE,
    author JSONB NOT NULL CHECK (author ->> 'type' IN ('admin', 'cli')),
    posted_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
-- An alert's updates, oldest first.
CREATE INDEX alert_updates_alert_idx ON alert_updates (alert_id, posted_at, id);

----------------------------------------------------------------------------
-- Admin dashboard sign-ins
----------------------------------------------------------------------------

-- Sign-ins to the admin dashboard, which auth serves on its admin listener. An
-- administrator's passkey session on auth hands off to the dashboard with a
-- single-use code, which the dashboard redeems for a session of its own. Only
-- digests of the code and of the dashboard's cookies are stored.
CREATE TABLE dashboard_handoffs (
    code_digest BYTEA PRIMARY KEY,
    -- The auth session handing off, whose sign-in the dashboard session rests on.
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    -- The digest of the dashboard's state cookie in the browser that asked, which
    -- must present it to redeem the code.
    state_digest BYTEA NOT NULL,
    -- The dashboard path the browser goes on to once signed in.
    return_to TEXT NOT NULL,
    -- When a passkey confirmed the handoff, in a ceremony started for it.
    confirmed_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX dashboard_handoffs_session_idx ON dashboard_handoffs (session_id);

-- The dashboard's own sessions. Each lasts while its auth session does, and
-- keeps its own passkey confirmation: one done for auth's pages never counts.
-- It also ends after 30 minutes without a request, and 12 hours after its
-- sign-in however busy, so an unattended browser or a stolen cookie stops
-- working within a working day. Auth sets `expires_at` on each sign-in, and
-- refreshes `last_used_at` on each request.
CREATE TABLE dashboard_sessions (
    secret_digest BYTEA PRIMARY KEY,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    confirmed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX dashboard_sessions_session_idx ON dashboard_sessions (session_id);
-- Cleanup deletes the sessions that ended.
CREATE INDEX dashboard_sessions_expires_idx ON dashboard_sessions (expires_at);

----------------------------------------------------------------------------
-- Function search paths
----------------------------------------------------------------------------

-- These functions look tables up in the schema this migration runs in, with
-- pg_temp last: public when deployed, a test's own schema in tests. The others
-- pin their search path above.
DO $$
DECLARE
    guard TEXT;
BEGIN
    FOREACH guard IN ARRAY ARRAY[
        'guard_personal_data_insert', 'guard_seal_order', 'guard_new_checkpoint',
        'limit_passkeys'
    ] LOOP
        EXECUTE format(
            'ALTER FUNCTION %I() SET search_path = pg_catalog, %I, pg_temp',
            guard, current_schema()
        );
    END LOOP;
END $$;

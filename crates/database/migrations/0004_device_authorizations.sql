-- The device authorization grant (RFC 8628): a device without a browser, such
-- as a command-line tool, shows its user a code, and polls the token endpoint
-- until the user approves it on auth's device page from another device.
--
-- A device authorization holds two codes, each stored only as an HMAC under the
-- token digest key named by `digest_key`. The device polls with the device
-- code, whose secret `secret_digest` covers. The user types the user code,
-- which `user_code_digest` covers: it is short, so only its digest can find the
-- row, and guesses are throttled.
--
-- An approval names the account and the session it creates for the device,
-- which the constraint below ties to the status. `poll_interval` is how many
-- seconds the device must leave between two polls; a poll that comes sooner
-- than `last_polled_at` allows adds five seconds to it.
--
-- The previous release knows neither the table nor the column, and writes
-- neither: it keeps working.
CREATE TABLE device_authorizations (
    id UUID PRIMARY KEY,
    secret_digest BYTEA NOT NULL UNIQUE,
    user_code_digest BYTEA NOT NULL UNIQUE,
    digest_key TEXT NOT NULL,
    client_id TEXT NOT NULL REFERENCES oidc_clients(id),
    scopes JSONB NOT NULL,
    audience TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'approved', 'denied', 'redeemed')),
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    session_id UUID REFERENCES sessions(id) ON DELETE CASCADE,
    poll_interval INTEGER NOT NULL CHECK (poll_interval > 0),
    last_polled_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT device_authorizations_approved_by CHECK (
        (status IN ('approved', 'redeemed')) = (user_id IS NOT NULL AND session_id IS NOT NULL)
        AND (user_id IS NULL) = (session_id IS NULL)
    )
);
-- Cleanup finds the authorizations that expired.
CREATE INDEX device_authorizations_expiry_idx ON device_authorizations (expires_at);
-- Purging an account, or deleting an ended session, deletes the
-- authorizations it approved.
CREATE INDEX device_authorizations_user_idx ON device_authorizations (user_id)
    WHERE user_id IS NOT NULL;
CREATE INDEX device_authorizations_session_idx ON device_authorizations (session_id)
    WHERE session_id IS NOT NULL;

-- The client a session belongs to, when the session is a device's: one that a
-- device authorization created for the client that asked. Such a session has no
-- browser cookie, so only that client's tokens use it. NULL for a browser's
-- session, which every client the browser signs in to shares.
ALTER TABLE sessions ADD COLUMN client_id TEXT REFERENCES oidc_clients(id);

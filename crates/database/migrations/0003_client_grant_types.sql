-- The grants a client may use at the token endpoint, under their RFC 7591 name:
-- a JSON array of grant type names, such as `["authorization_code",
-- "refresh_token"]`. It replaces `allow_refresh`, which said only whether a
-- client got refresh tokens: a client's other grants, such as the device grant,
-- need the same per-client switch, and one list holds them all.
--
-- Every client keeps the authorization code, and `refresh_token` where
-- `allow_refresh` was set, so no client's behaviour changes.
--
-- Dropping `allow_refresh` breaks the expand-then-contract rule: the previous
-- release reads it in every client lookup, so it fails until its replacement
-- is healthy. There are no real users yet, so that window costs less than
-- keeping two columns in step.
ALTER TABLE oidc_clients
    ADD COLUMN grant_types JSONB NOT NULL DEFAULT '["authorization_code"]'
    CHECK (jsonb_typeof(grant_types) = 'array' AND jsonb_array_length(grant_types) > 0);

UPDATE oidc_clients
SET grant_types = '["authorization_code", "refresh_token"]'
WHERE allow_refresh;

ALTER TABLE oidc_clients DROP COLUMN allow_refresh;

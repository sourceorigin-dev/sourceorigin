-- APIs as resources (RFC 8707). A resource is an API that access tokens are
-- issued for: its identifier, an absolute https URI, is the `aud` of those
-- tokens. Each resource defines its own scopes, described for the consent
-- page, and names the clients that may ask for tokens for it, each with the
-- scopes it may request.
--
-- owner_id is the account that registered the resource, which alone manages it
-- beside operators and administrators; NULL for a resource operators
-- registered. Purging the account deletes its resources.
--
-- Grants name their audience as text, never by key: the client's own ID, or a
-- resource's identifier. So deleting a resource, or a client's access to one,
-- revokes the refresh families and consents that name it in the statement that
-- deletes it (`ResourceRepository`), and no row here is referenced by a grant.
CREATE TABLE resources (
    id UUID PRIMARY KEY,
    identifier TEXT NOT NULL UNIQUE CHECK (char_length(identifier) <= 255),
    display_name TEXT NOT NULL,
    owner_id UUID REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Lists an owner's resources, and counts them for the quota.
CREATE INDEX resources_owner_idx ON resources (owner_id);

CREATE TABLE resource_scopes (
    resource_id UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    PRIMARY KEY (resource_id, name)
);

-- scopes is a JSON array of the resource's scope names the client may request.
-- Clients are tombstoned, never deleted, so deleting one removes its access in
-- the same statement rather than by this cascade.
CREATE TABLE client_resource_access (
    client_id TEXT NOT NULL REFERENCES oidc_clients(id) ON DELETE CASCADE,
    resource_id UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    scopes JSONB NOT NULL,
    granted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (client_id, resource_id)
);
-- Lists the clients a resource admits, and cascades its deletion.
CREATE INDEX client_resource_access_resource_idx ON client_resource_access (resource_id);

-- A client's audiences were a list that only ever held its own ID. Its own ID
-- stays an audience it may always ask for, and the others are now the
-- resources it has access to, so the column goes. Its allowed scopes keep only
-- the scopes auth itself defines: an API's scopes are its resource's.
--
-- Dropping the column breaks the expand-then-contract rule: the previous
-- release reads it in every client lookup, so it fails until its replacement
-- is healthy. There are no real users yet, so that window costs less than
-- keeping a column nothing writes.
ALTER TABLE oidc_clients DROP COLUMN allowed_audiences;

UPDATE oidc_clients c
SET allowed_scopes = COALESCE(
    (
        SELECT jsonb_agg(scope)
        FROM jsonb_array_elements_text(c.allowed_scopes) AS s(scope)
        WHERE scope IN ('openid', 'profile', 'email', 'offline_access')
    ),
    '["openid"]'
);

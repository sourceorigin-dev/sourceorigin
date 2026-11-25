-- The client credentials grant (RFC 6749 §4.4) gives a client a token for
-- itself, on its own credentials. A public client has none: anyone knowing its
-- ID would get its tokens. `ClientKind::grant_types` refuses the grant for a
-- public client before any SQL runs, and this constraint holds it for every
-- writer.
ALTER TABLE oidc_clients
    ADD CONSTRAINT oidc_clients_client_credentials_confidential
    CHECK (client_type = 'confidential' OR NOT grant_types ? 'client_credentials');

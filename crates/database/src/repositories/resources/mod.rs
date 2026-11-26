//! APIs registered as resources (RFC 8707): their scopes, and the clients that
//! may ask for tokens for them. Grants name a resource by its identifier, as
//! text, so what ends a client's access also ends the grants that relied on it,
//! in the same statement.

use crate::{
    DatabaseError,
    repositories::{JsonArray, RepositoryExecutor, ResourceRepository, Unsigned},
};
use sourceorigin_common::{
    ClientId, ResourceId, UserId,
    admin::{GrantedResource, Resource, ResourceAccess, ResourceDetail, ScopeDefinition},
    clients::{ClientOwner, DisplayName, ScopeDescription},
    protocol::resource::{ResourceIdentifier, ResourceScope},
};
use sqlx::types::Json;

/// A [`query_record!`] returning [`Resource`]s, read as `r`.
macro_rules! resources {
    ($($query:tt)*) => {
        query_record!(
            Resource,
            r#"r.id AS "id: ResourceId", r.identifier, r.display_name,
            r.owner_id AS "owner_id: UserId", r.created_at, r.updated_at"#,
            $($query)*
        )
    };
}

impl ResourceRepository<'_> {
    /// Registers the resource `identifier`, named `name`, managed by `owner`.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::Conflict`] if the identifier is taken, or another
    /// [`DatabaseError`] if the query fails.
    pub async fn create(
        &mut self,
        id: ResourceId,
        identifier: &ResourceIdentifier,
        name: &DisplayName,
        owner: ClientOwner,
    ) -> Result<(), DatabaseError> {
        run!(
            self,
            execute,
            sqlx::query!(
                "INSERT INTO resources (id, identifier, display_name, owner_id)
                VALUES ($1, $2, $3, $4)",
                id.as_uuid(),
                identifier.as_str(),
                name.as_str(),
                owner.user_id().map(UserId::as_uuid),
            )
        )?;
        Ok(())
    }

    /// Finds the resource `id`, and locks it until the transaction ends, so
    /// changes to its scopes and to the clients it admits take turns.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::OutsideTransaction`] outside a transaction, or
    /// another [`DatabaseError`] if the query fails or the row is invalid.
    pub async fn lock(&mut self, id: ResourceId) -> Result<Option<Resource>, DatabaseError> {
        self.executor.require_transaction()?;
        run!(
            self,
            fetch_optional,
            resources!(
                "SELECT",
                "FROM resources r WHERE r.id = $1 FOR UPDATE",
                id.as_uuid()
            )
        )
    }

    /// Finds the resource whose identifier is `identifier`.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or the row is invalid.
    pub async fn find_by_identifier(
        &mut self,
        identifier: &str,
    ) -> Result<Option<Resource>, DatabaseError> {
        run!(
            self,
            fetch_optional,
            resources!(
                "SELECT",
                "FROM resources r WHERE r.identifier = $1",
                identifier
            )
        )
    }

    /// Lists every resource, or those `owner` manages, by identifier.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or a row is invalid.
    pub async fn list(&mut self, owner: Option<UserId>) -> Result<Vec<Resource>, DatabaseError> {
        run!(
            self,
            fetch_all,
            resources!(
                "SELECT",
                "FROM resources r
                WHERE $1::uuid IS NULL OR r.owner_id = $1
                ORDER BY r.identifier",
                owner.map(UserId::as_uuid)
            )
        )
    }

    /// How many resources `owner` manages.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn count_owned(&mut self, owner: UserId) -> Result<u64, DatabaseError> {
        Ok(run!(
            self,
            fetch_one,
            sqlx::query_scalar!(
                r#"SELECT count(*) AS "count!: Unsigned" FROM resources WHERE owner_id = $1"#,
                owner.as_uuid(),
            )
        )?
        .into())
    }

    /// The resource `id` with the scopes it defines and the clients it admits,
    /// or `None` if there is none.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if a query fails or a row is invalid.
    pub async fn detail(
        &mut self,
        id: ResourceId,
    ) -> Result<Option<ResourceDetail>, DatabaseError> {
        let Some(resource) = run!(
            self,
            fetch_optional,
            resources!("SELECT", "FROM resources r WHERE r.id = $1", id.as_uuid())
        )?
        else {
            return Ok(None);
        };
        let scopes = self.scopes(id).await?;
        let access = run!(
            self,
            fetch_all,
            sqlx::query_as!(
                ResourceAccess,
                r#"SELECT
                    a.client_id AS "client_id: ClientId", c.display_name AS client_name,
                    a.scopes AS "scopes: JsonArray<String>", a.granted_at
                FROM client_resource_access a
                JOIN oidc_clients c ON c.id = a.client_id
                WHERE a.resource_id = $1
                ORDER BY a.client_id"#,
                id.as_uuid(),
            )
        )?;
        Ok(Some(ResourceDetail {
            resource,
            scopes,
            access,
        }))
    }

    /// The resource whose identifier is `audience`, with those of `scopes` it
    /// defines, as a consent page describes them, or `None` when the audience is
    /// no resource, as a client's own ID is.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails or the row is invalid.
    pub async fn granted(
        &mut self,
        audience: &str,
        scopes: &[String],
    ) -> Result<Option<GrantedResource>, DatabaseError> {
        run!(
            self,
            fetch_optional,
            sqlx::query_as!(
                GrantedResource,
                r#"SELECT
                    r.identifier, r.display_name,
                    COALESCE(
                        (
                            SELECT jsonb_agg(
                                jsonb_build_object('name', s.name, 'description', s.description)
                                ORDER BY s.name
                            )
                            FROM resource_scopes s
                            WHERE s.resource_id = r.id AND s.name = ANY($2)
                        ),
                        '[]'::jsonb
                    ) AS "scopes!: JsonArray<ScopeDefinition>"
                FROM resources r
                WHERE r.identifier = $1"#,
                audience,
                scopes,
            )
        )
    }

    /// The scopes the resource `id` defines, by name.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn scopes(&mut self, id: ResourceId) -> Result<Vec<ScopeDefinition>, DatabaseError> {
        run!(
            self,
            fetch_all,
            sqlx::query_as!(
                ScopeDefinition,
                "SELECT name, description FROM resource_scopes
                WHERE resource_id = $1
                ORDER BY name",
                id.as_uuid(),
            )
        )
    }

    /// Renames the resource `id`.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::NotFound`] if there is none, or another
    /// [`DatabaseError`] if the query fails.
    pub async fn rename(
        &mut self,
        id: ResourceId,
        name: &DisplayName,
    ) -> Result<(), DatabaseError> {
        run!(
            self,
            required,
            sqlx::query!(
                "UPDATE resources SET display_name = $2, updated_at = now() WHERE id = $1",
                id.as_uuid(),
                name.as_str(),
            )
        )
    }

    /// Deletes the resource `id`, with its scopes and the access clients had to
    /// it, revokes the refresh families issued for it, and removes the consents
    /// users gave for it: its identifier may be registered again, and must then
    /// start from nothing. Returns whether there was one.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn delete(&mut self, id: ResourceId) -> Result<bool, DatabaseError> {
        Ok(run!(
            self,
            fetch_optional,
            sqlx::query_scalar!(
                "WITH deleted AS (
                    DELETE FROM resources WHERE id = $1 RETURNING identifier
                ),
                families AS (
                    UPDATE refresh_token_families f
                    SET revoked_at = now()
                    FROM deleted
                    WHERE f.audience = deleted.identifier AND f.revoked_at IS NULL
                ),
                consents AS (
                    DELETE FROM oidc_consents c USING deleted
                    WHERE c.audience = deleted.identifier
                )
                SELECT identifier FROM deleted",
                id.as_uuid(),
            )
        )?
        .is_some())
    }

    /// Defines the scope `name` of the resource `id`, described as
    /// `description`, or describes it anew. Returns whether it is new.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the resource does not exist or the query
    /// fails.
    pub async fn put_scope(
        &mut self,
        id: ResourceId,
        name: &ResourceScope,
        description: &ScopeDescription,
    ) -> Result<bool, DatabaseError> {
        run!(
            self,
            fetch_one,
            sqlx::query_scalar!(
                r#"WITH put AS (
                    INSERT INTO resource_scopes (resource_id, name, description)
                    VALUES ($1, $2, $3)
                    ON CONFLICT (resource_id, name) DO UPDATE
                    SET description = EXCLUDED.description
                    RETURNING (xmax = 0) AS inserted
                ),
                touched AS (
                    UPDATE resources SET updated_at = now() WHERE id = $1
                )
                SELECT inserted AS "inserted!" FROM put"#,
                id.as_uuid(),
                name.as_str(),
                description.as_str(),
            )
        )
    }

    /// Removes the scope `name` from the resource `id`, and from the scopes
    /// every client may request of it. Returns whether it was defined.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn remove_scope(
        &mut self,
        id: ResourceId,
        name: &str,
    ) -> Result<bool, DatabaseError> {
        Ok(run!(
            self,
            fetch_optional,
            sqlx::query_scalar!(
                "WITH removed AS (
                    DELETE FROM resource_scopes WHERE resource_id = $1 AND name = $2
                    RETURNING name
                ),
                access AS (
                    UPDATE client_resource_access a
                    SET scopes = a.scopes - removed.name
                    FROM removed
                    WHERE a.resource_id = $1
                ),
                touched AS (
                    UPDATE resources r SET updated_at = now() FROM removed WHERE r.id = $1
                )
                SELECT name FROM removed",
                id.as_uuid(),
                name,
            )
        )?
        .is_some())
    }

    /// Lets `client` ask for tokens for the resource `id` with `scopes`, which
    /// replace any it could request before.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the client or the resource does not exist, or
    /// the query fails.
    pub async fn grant_access(
        &mut self,
        client: &ClientId,
        id: ResourceId,
        scopes: &[ResourceScope],
    ) -> Result<(), DatabaseError> {
        let scopes: Vec<&str> = scopes.iter().map(ResourceScope::as_str).collect();
        run!(
            self,
            execute,
            sqlx::query!(
                "INSERT INTO client_resource_access (client_id, resource_id, scopes)
                VALUES ($1, $2, $3)
                ON CONFLICT (client_id, resource_id) DO UPDATE SET scopes = EXCLUDED.scopes",
                client.as_str(),
                id.as_uuid(),
                Json(scopes) as _,
            )
        )?;
        Ok(())
    }

    /// Ends `client`'s access to the resource `id`, and revokes the refresh
    /// families the client holds for it. Returns whether it had access.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] if the query fails.
    pub async fn revoke_access(
        &mut self,
        client: &ClientId,
        id: ResourceId,
    ) -> Result<bool, DatabaseError> {
        Ok(run!(
            self,
            fetch_optional,
            sqlx::query_scalar!(
                "WITH revoked AS (
                    DELETE FROM client_resource_access a
                    USING resources r
                    WHERE a.client_id = $1 AND a.resource_id = $2 AND r.id = a.resource_id
                    RETURNING r.identifier
                ),
                families AS (
                    UPDATE refresh_token_families f
                    SET revoked_at = now()
                    FROM revoked
                    WHERE f.client_id = $1
                    AND f.audience = revoked.identifier
                    AND f.revoked_at IS NULL
                )
                SELECT identifier FROM revoked",
                client.as_str(),
                id.as_uuid(),
            )
        )?
        .is_some())
    }
}

/// Database tests for resources, their scopes and the grants that end with them.
#[cfg(test)]
mod tests;

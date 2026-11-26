//! Resources against a real database: what a client may be granted, and the
//! grants that end with a resource or with a client's access to it.
use crate::{
    Database, DatabaseTransaction, InsertUser,
    repositories::{ResourceRepository, UserRepository},
};
use sourceorigin_common::{
    ClientId, ResourceId, UserId,
    clients::ClientOwner,
    protocol::resource::{ResourceIdentifier, ResourceScope},
};
use sqlx::PgPool;

/// A client with a fresh ID, and a refresh family and a consent of a new
/// account for `audience`, whose family ID is returned with the client.
async fn granted(pool: &PgPool, audience: &str) -> (ClientId, uuid::Uuid) {
    let client: ClientId = format!("api_{}", UserId::new()).parse().expect("client ID");
    let user = account(pool).await;
    let (session, family) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
    sqlx::query!(
        "WITH client AS (
            INSERT INTO oidc_clients (id, display_name, allowed_scopes) VALUES ($1, 'API test', '[]')
        ), session AS (
            INSERT INTO sessions (
                id, user_id, authentication_methods, authenticated_at, expires_at, last_seen_at
            ) VALUES ($3, $2, '[]', now(), now() + interval '1 day', now())
        ), consent AS (
            INSERT INTO oidc_consents (user_id, client_id, audience, scopes)
            VALUES ($2, $1, $5, '[]')
        )
        INSERT INTO refresh_token_families (
            id, user_id, client_id, session_id, audience, scopes, authentication
        ) VALUES ($4, $2, $1, $3, $5, '[]', '{}')",
        client.as_str(),
        user.as_uuid(),
        session,
        family,
        audience,
    )
    .execute(pool)
    .await
    .expect("a grant");
    (client, family)
}

/// A new account.
async fn account(pool: &PgPool) -> UserId {
    let user = InsertUser::named("resource");
    let user = UserRepository::from_pool(pool).insert(&user).await;
    user.expect("user").id
}

/// How many consents name `audience`.
async fn consents(pool: &PgPool, audience: &str) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM oidc_consents WHERE audience = $1"#,
        audience
    )
    .fetch_one(pool)
    .await
    .expect("consents")
}

/// Whether refresh family `id` is revoked.
async fn revoked(pool: &PgPool, id: uuid::Uuid) -> bool {
    sqlx::query_scalar!(
        r#"SELECT revoked_at IS NOT NULL AS "revoked!" FROM refresh_token_families WHERE id = $1"#,
        id
    )
    .fetch_one(pool)
    .await
    .expect("family")
}

/// A new resource `owner` manages, with a unique identifier, defining
/// `notes:read`.
async fn resource(pool: &PgPool, owner: ClientOwner) -> (ResourceId, ResourceIdentifier) {
    let id = ResourceId::new();
    let identifier: ResourceIdentifier = format!("https://{id}.example/api")
        .parse()
        .expect("identifier");
    let mut resources = ResourceRepository::from_pool(pool);
    resources
        .create(id, &identifier, &"Notes API".parse().expect("name"), owner)
        .await
        .expect("resource");
    let read: ResourceScope = "notes:read".parse().expect("scope");
    assert!(
        resources
            .put_scope(id, &read, &"Read your notes".parse().expect("description"))
            .await
            .expect("scope")
    );
    (id, identifier)
}

/// Ending a client's access to a resource revokes the refresh families that
/// client holds for it, and nobody else's; removing a scope takes it from what
/// every client may request; and deleting the resource revokes every family and
/// removes every consent for it, so its identifier starts from nothing again.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn grants_end_with_the_access_and_the_resource_they_rely_on() {
    let pool = Database::test_pool().await;
    let (id, identifier) = resource(&pool, ClientOwner::Operator).await;
    let (first, first_family) = granted(&pool, identifier.as_str()).await;
    let (second, second_family) = granted(&pool, identifier.as_str()).await;
    let mut resources = ResourceRepository::from_pool(&pool);
    let read: ResourceScope = "notes:read".parse().expect("scope");
    for client in [&first, &second] {
        resources
            .grant_access(client, id, std::slice::from_ref(&read))
            .await
            .expect("access");
    }

    assert!(resources.revoke_access(&first, id).await.expect("revoke"));
    assert!(!resources.revoke_access(&first, id).await.expect("again"));
    assert!(revoked(&pool, first_family).await);
    assert!(!revoked(&pool, second_family).await);

    assert!(
        resources
            .remove_scope(id, "notes:read")
            .await
            .expect("scope")
    );
    let detail = resources
        .detail(id)
        .await
        .expect("detail")
        .expect("resource");
    assert_eq!(detail.scopes, []);
    assert_eq!(detail.access.len(), 1);
    assert_eq!(detail.access[0].scopes, Vec::<String>::new());

    assert!(resources.delete(id).await.expect("delete"));
    assert!(revoked(&pool, second_family).await);
    assert_eq!(consents(&pool, identifier.as_str()).await, 0);
    assert!(
        resources
            .find_by_identifier(identifier.as_str())
            .await
            .expect("lookup")
            .is_none()
    );
}

/// Purging an account deletes the resources it owns, with their scopes and the
/// access clients had to them, and ends what other accounts granted for them:
/// their refresh families are revoked and their consents removed.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn purging_an_owner_ends_its_resources_and_the_grants_for_them() {
    let pool = Database::test_pool().await;
    let owner = account(&pool).await;
    let (id, identifier) = resource(&pool, ClientOwner::Account(owner)).await;
    let (client, family) = granted(&pool, identifier.as_str()).await;
    let read: ResourceScope = "notes:read".parse().expect("scope");
    let mut resources = ResourceRepository::from_pool(&pool);
    resources
        .grant_access(&client, id, std::slice::from_ref(&read))
        .await
        .expect("access");

    let mut tx = DatabaseTransaction::new(pool.begin().await.expect("transaction"));
    tx.deletions()
        .schedule(owner, chrono::Utc::now())
        .await
        .expect("schedule");
    assert!(tx.deletions().purge(owner).await.expect("purge"));
    tx.commit().await.expect("commit");

    assert!(resources.detail(id).await.expect("detail").is_none());
    let left = sqlx::query_scalar!(
        r#"SELECT
            (SELECT count(*) FROM resource_scopes WHERE resource_id = $1)
            + (SELECT count(*) FROM client_resource_access WHERE resource_id = $1) AS "count!""#,
        id.as_uuid()
    )
    .fetch_one(&pool)
    .await
    .expect("rows");
    assert_eq!(left, 0);
    assert!(revoked(&pool, family).await);
    assert_eq!(consents(&pool, identifier.as_str()).await, 0);
}

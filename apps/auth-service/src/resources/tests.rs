//! Resources against a real database: owners register and change only their
//! own, once confirmed, within their quota, and a grant names scopes the
//! resource defines and a client that exists.
use super::*;
use crate::{
    account::{AccountError, AccountService},
    security::SessionError,
    tests::{Browser, Replicas, api, change_resources},
};
use sourceorigin_common::{AuthenticationMethod, clients::ClientKind};

/// A resource an owner registers, with an identifier of its own.
fn new(name: &str) -> NewResource {
    let identifier = format!("https://{}.example/api", ResourceId::new());
    NewResource::typed(&identifier, name).expect("resource")
}

/// An owner registers a resource once confirmed with a passkey, finds it, and
/// changes it; another account finds none by its ID. A name after a reserved
/// brand, an identifier already registered and an eleventh resource are refused.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn owners_register_and_find_only_their_own() {
    let database = Replicas::database().await;
    let service = AccountService::new(database.clone());
    let browser = Browser::new_account(&database).await;
    let owner = browser.owner(&database).await;
    let federation = AuthenticationMethod::Federation;
    let social = Browser::sign_in_with(&database, browser.user_id, federation).await;
    let unconfirmed = service
        .create_resource(&social.owner(&database).await, &new("Notes API"))
        .await;
    assert!(
        matches!(
            unconfirmed,
            Err(AccountError::Owner(SessionError::StepUpRequired { .. }))
        ),
        "{unconfirmed:?}"
    );

    let notes = new("Notes API");
    let id = service
        .create_resource(&owner, &notes)
        .await
        .expect("register");
    let listed = service.resources(&owner).await.expect("resources");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].identifier, notes.identifier.as_str());
    assert_eq!(listed[0].owner_id, Some(browser.user_id));
    let registered = service
        .owned_resource(owner.clone(), &id.to_string())
        .await
        .expect("the owner's");
    let client = browser
        .register_app(&database, ClientKind::Public, "https://notes.example")
        .await;
    for edit in [
        ResourceEdit::put_scope("notes:read", "Read your notes"),
        ResourceEdit::grant(client.as_str(), "notes:read"),
        ResourceEdit::rename("Notes"),
    ] {
        let edit = edit.expect("edit");
        service
            .edit_resource(&registered, edit)
            .await
            .expect("edit");
    }
    let detail = database.resources().detail(id).await.expect("detail");
    let detail = detail.expect("resource");
    assert_eq!(detail.resource.display_name, "Notes");
    assert_eq!(detail.scopes[0].description, "Read your notes");
    assert_eq!(detail.access[0].client_id, client);

    let stranger = Browser::new_account(&database).await;
    let stranger = stranger.owner(&database).await;
    for id in [id.to_string(), "notes".to_owned()] {
        let found = service.owned_resource(stranger.clone(), &id).await;
        assert!(
            matches!(
                found,
                Err(AccountError::Resource(OwnedResourceError::NotFound))
            ),
            "{id}"
        );
    }

    let refused = |result: Result<ResourceId, AccountError>| match result {
        Err(AccountError::Resource(OwnedResourceError::Registry(error))) => error,
        result => panic!("{result:?}"),
    };
    let reserved = new("SourceOrigin API");
    let reserved = service.create_resource(&owner, &reserved).await;
    assert!(matches!(
        refused(reserved),
        ResourceRegistryError::ReservedName(_)
    ));
    let again = service.create_resource(&stranger, &notes).await;
    assert!(matches!(
        refused(again),
        ResourceRegistryError::Exists { .. }
    ));
    for _ in 1..MAX_OWNED_RESOURCES {
        service
            .create_resource(&owner, &new("Notes API"))
            .await
            .expect("within the quota");
    }
    let over = service.create_resource(&owner, &new("Notes API")).await;
    assert!(matches!(
        refused(over),
        ResourceRegistryError::QuotaExceeded
    ));
}

/// A grant names scopes the resource defines and a client that is registered,
/// a revocation an access that exists, and every edit a resource that does. A
/// resource defines at most [`MAX_RESOURCE_SCOPES`] scopes, though describing
/// one of them anew still works.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn edits_name_what_exists() {
    let database = Replicas::database().await;
    let client = crate::tests::registration(ClientKind::Public, "https://notes.example");
    crate::tests::register(&database, &client)
        .await
        .expect("client");
    let (id, _) = api(&database, &client.id, "notes:read").await;
    let edit = async |id: ResourceId, edit: Result<ResourceEdit, InvalidResourceInput>| {
        let edit = edit.expect("edit");
        change_resources(&database, async |resources| resources.edit(id, edit).await).await
    };

    let undefined = edit(id, ResourceEdit::grant(client.id.as_str(), "notes:delete")).await;
    assert!(matches!(
        undefined,
        Err(ResourceRegistryError::UnknownScope(scope)) if scope == "notes:delete"
    ));
    let nobody = edit(id, ResourceEdit::grant("unregistered", "notes:read")).await;
    assert!(matches!(
        nobody,
        Err(ResourceRegistryError::UnknownClient(_))
    ));
    let never = edit(id, ResourceEdit::revoke("unregistered")).await;
    assert!(matches!(never, Err(ResourceRegistryError::NoAccess(_))));
    let removed = edit(id, Ok(ResourceEdit::RemoveScope("notes:delete".into()))).await;
    assert!(matches!(
        removed,
        Err(ResourceRegistryError::UnknownScope(_))
    ));
    let missing = edit(ResourceId::new(), ResourceEdit::rename("Notes")).await;
    assert!(matches!(missing, Err(ResourceRegistryError::Unknown)));

    for number in 2..MAX_RESOURCE_SCOPES {
        let scope = ResourceEdit::put_scope(&format!("notes:{number}"), "Do something");
        edit(id, scope).await.expect("within the limit");
    }
    let over = edit(id, ResourceEdit::put_scope("notes:more", "Do more")).await;
    assert!(matches!(over, Err(ResourceRegistryError::TooManyScopes)));
    edit(id, ResourceEdit::put_scope("notes:read", "Read every note"))
        .await
        .expect("describe anew");
}

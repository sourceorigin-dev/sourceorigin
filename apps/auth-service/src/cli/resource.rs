//! Registering, inspecting and changing the APIs registered as resources.

use super::{
    CliError, Confirm, Console, Why,
    output::{Cell, Joined, Style, report},
};
use crate::resources::ResourceEdit;
use clap::Subcommand;
use sourceorigin_common::{
    ClientId,
    admin::{Resource, ResourceDetail},
    clients::{DisplayName, ScopeDescription},
    protocol::resource::{ResourceIdentifier, ResourceScope},
};

/// Registering, inspecting and changing the APIs access tokens are issued for.
#[derive(Subcommand)]
pub(super) enum ResourceCommand {
    /// Register an API as a resource operators manage. Clients name it in the
    /// `resource` parameter, and its access tokens carry its identifier as `aud`.
    Create {
        /// The identifier: an absolute https URI, written as the API checks it,
        /// e.g. `https://notes.example/api`. It cannot change later.
        identifier: ResourceIdentifier,
        /// The name consent screens show.
        #[arg(long)]
        name: DisplayName,
        /// Why.
        #[command(flatten)]
        why: Why,
    },
    /// List every resource.
    List,
    /// Show a resource, with the scopes it defines and the clients it admits.
    Show {
        /// The resource's identifier.
        identifier: ResourceIdentifier,
    },
    /// Give a resource another name.
    Rename {
        /// The resource's identifier.
        identifier: ResourceIdentifier,
        /// The new name consent screens show.
        #[arg(long)]
        name: DisplayName,
        /// Why.
        #[command(flatten)]
        why: Why,
    },
    /// Delete a resource: no client gets tokens for it any more, and every
    /// refresh token and consent for it ends.
    Delete {
        /// The resource's identifier.
        identifier: ResourceIdentifier,
        /// Why.
        #[command(flatten)]
        why: Why,
        /// Confirmation.
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Define or remove the scopes of a resource.
    #[command(subcommand)]
    Scope(ScopeCommand),
    /// Let a client ask for a resource's tokens, with these scopes of it, which
    /// replace any it could request before.
    Grant {
        /// The client's ID.
        client: ClientId,
        /// The resource's identifier.
        identifier: ResourceIdentifier,
        /// A scope of the resource the client may request; repeat for several.
        /// With none, its tokens for the resource carry only Origin Auth's own
        /// scopes.
        #[arg(long = "scope")]
        scopes: Vec<ResourceScope>,
        /// Why.
        #[command(flatten)]
        why: Why,
    },
    /// End a client's access to a resource, with its refresh tokens for it.
    Revoke {
        /// The client's ID.
        client: ClientId,
        /// The resource's identifier.
        identifier: ResourceIdentifier,
        /// Why.
        #[command(flatten)]
        why: Why,
    },
}

/// The scopes a resource defines, which clients given them may request for it.
#[derive(Subcommand)]
pub(super) enum ScopeCommand {
    /// Define a scope of a resource, or describe it anew.
    Add {
        /// The resource's identifier.
        identifier: ResourceIdentifier,
        /// The scope, as clients request it, e.g. `notes:read`.
        scope: ResourceScope,
        /// What it lets a client do, as the consent screen says it to the user,
        /// e.g. "Read your notes".
        #[arg(long)]
        description: ScopeDescription,
        /// Why.
        #[command(flatten)]
        why: Why,
    },
    /// Remove a scope from a resource, and from what every client may request of
    /// it. A refresh token that holds it is refused at its next use.
    Remove {
        /// The resource's identifier.
        identifier: ResourceIdentifier,
        /// The scope, as defined.
        scope: String,
        /// Why.
        #[command(flatten)]
        why: Why,
    },
}

impl ResourceCommand {
    /// Runs this command as the operator at this terminal. Each change commits
    /// with its event before anything is printed.
    pub(super) async fn run(self, console: &Console) -> Result<(), CliError> {
        let Console {
            service,
            operator,
            out,
            ..
        } = console;
        let (identifier, edit, why, done) = match self {
            Self::Create {
                identifier,
                name,
                why,
            } => {
                service
                    .create_resource(operator, &identifier, &name, why.reason)
                    .await?;
                return out.info(format_args!("Registered resource {identifier}."));
            }
            Self::List => return out.emit(&service.list_resources(operator).await?[..]),
            Self::Show { identifier } => {
                let found = service.find_resource(operator, &identifier).await?;
                return out.emit(&service.resource_detail(operator, found.id).await?);
            }
            Self::Delete {
                identifier,
                why,
                confirm,
            } => {
                confirm.ask(
                    out,
                    &format!(
                        "Delete resource {identifier}: every refresh token and consent for it ends"
                    ),
                )?;
                let found = service.find_resource(operator, &identifier).await?;
                service
                    .delete_resource(operator, found.id, why.reason)
                    .await?;
                return out.info(format_args!("Deleted resource {identifier}."));
            }
            Self::Rename {
                identifier,
                name,
                why,
            } => (identifier, ResourceEdit::Rename(name), why, "Renamed"),
            Self::Scope(ScopeCommand::Add {
                identifier,
                scope,
                description,
                why,
            }) => (
                identifier,
                ResourceEdit::PutScope(scope, description),
                why,
                "Defined the scope of",
            ),
            Self::Scope(ScopeCommand::Remove {
                identifier,
                scope,
                why,
            }) => (
                identifier,
                ResourceEdit::RemoveScope(scope),
                why,
                "Removed the scope from",
            ),
            Self::Grant {
                client,
                identifier,
                scopes,
                why,
            } => (
                identifier,
                ResourceEdit::Grant(client, scopes),
                why,
                "Granted the client access to",
            ),
            Self::Revoke {
                client,
                identifier,
                why,
            } => (
                identifier,
                ResourceEdit::Revoke(client),
                why,
                "Revoked the client's access to",
            ),
        };
        let found = service.find_resource(operator, &identifier).await?;
        service
            .edit_resource(operator, found.id, edit, why.reason)
            .await?;
        out.info(format_args!("{done} resource {identifier}."))
    }

    /// Who manages a resource: its owner's account ID, or a muted `operator`.
    fn owner(resource: &Resource) -> Cell {
        resource
            .owner_id
            .map_or_else(|| Cell::new("operator").style(Style::Muted), Cell::new)
    }
}

report!([Resource], self, out, {
    out.table(
        ["IDENTIFIER", "NAME", "OWNER", "REGISTERED"],
        "No resources.",
        self.iter().map(|resource| {
            [
                Cell::new(&resource.identifier),
                Cell::new(&resource.display_name),
                ResourceCommand::owner(resource),
                resource.created_at.into(),
            ]
        }),
    )
});

report!(ResourceDetail, self, out, {
    let resource = &self.resource;
    out.title(format_args!(
        "{} ({})",
        resource.display_name, resource.identifier
    ))?;
    out.fields(&[
        ("ID", Cell::new(resource.id)),
        ("Owner", ResourceCommand::owner(resource)),
        ("Registered", resource.created_at.into()),
        ("Changed", resource.updated_at.into()),
    ])?;
    out.section("Scopes")?;
    out.table(
        ["SCOPE", "DESCRIPTION"],
        "No scopes.",
        self.scopes
            .iter()
            .map(|scope| [Cell::new(&scope.name), Cell::new(&scope.description)]),
    )?;
    out.section("Clients with access")?;
    out.table(
        ["CLIENT", "SCOPES", "SINCE"],
        "No clients.",
        self.access.iter().map(|access| {
            [
                Cell::new(&access.client_id),
                Cell::new(Joined(&access.scopes, " ")),
                access.granted_at.into(),
            ]
        }),
    )
});

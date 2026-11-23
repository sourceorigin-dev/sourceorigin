//! axum integration, behind the `axum` feature.
//!
//! **Resource servers** add a [`BearerLayer`], then take [`Authenticated`], [`Scoped`]
//! or [`RequireAuthentication`] in handlers; a rejection answers with the RFC 6750 or 9470 challenge.
//!
//! ```no_run
//! use axum::{Json, Router, routing::get};
//! use sourceorigin_auth::axum::{BearerLayer, Scoped};
//! # use sourceorigin_auth::{Issuer, Resource};
//!
//! sourceorigin_auth::scopes! {
//!     /// Scopes of the notes API.
//!     pub mod notes {
//!         Read = "notes:read",
//!         Write = "notes:write",
//!     }
//! }
//!
//! async fn list(Scoped(token): Scoped<notes::Read>) -> Json<Vec<String>> {
//!     Json(vec![format!("notes of {:?}", token.subject())])
//! }
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let issuer = Issuer::discover("https://auth.sourceorigin.dev").await?;
//! let app: Router = Router::new()
//!     .route("/notes", get(list))
//!     .layer(BearerLayer::new(issuer.resource(Resource::new("notes-api")?)));
//! # Ok(()) }
//! ```
//!
//! **Apps that sign users in** mount [`LoginRoutes`] and its layer, then take [`SignedIn`]
//! or [`MaybeSignedIn`], which refuse an unsafe request from another origin;
//! [`CrossOrigin`] opts an endpoint out.
//!
//! ```no_run
//! use axum::{Router, routing::get};
//! use sourceorigin_auth::axum::{CookieKey, LoginRoutes, SignedIn};
//! # use sourceorigin_auth::{ClientConfig, Credentials, Issuer};
//!
//! async fn home(user: SignedIn) -> String {
//!     format!("Hello {}", user.name())
//! }
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! # let issuer = Issuer::discover("https://auth.sourceorigin.dev").await?;
//! # let config = ClientConfig {
//! #     client_id: "notes-web".parse()?,
//! #     credentials: Credentials::Secret(std::env::var("NOTES_CLIENT_SECRET")?.into()),
//! #     redirect_uri: "https://notes.sourceorigin.dev/auth/callback".parse()?,
//! # };
//! // The same key on every replica: `openssl rand -base64 32`.
//! let key = CookieKey::parse(&std::env::var("NOTES_COOKIE_KEY")?)?;
//! let auth = LoginRoutes::new(issuer.client(config), key)?
//!     .scopes("profile".parse()?)
//!     .after_login("/notes".parse()?);
//! let app: Router = Router::new()
//!     .route("/", get(home))
//!     .nest("/auth", auth.router())
//!     .layer(auth.layer());
//! # Ok(()) }
//! ```
//!
//! **Logout and step-up.** [`LoginRoutes::on_backchannel_logout`] ends sessions sooner than a
//! refresh. A page needing a stronger sign-in returns the [`StepUp`] from [`SignedIn::require`];
//! an API answers with [`SignedIn::step_up`].

/// Bearer token validation for resource servers.
mod bearer;
/// Server-side login routes and session extractors.
mod login;
/// Sealing the browser's cookies.
mod seal;
/// What the browser's cookies hold.
mod session;
#[cfg(test)]
mod tests;

pub use bearer::{
    AuthRejection, Authenticated, BearerLayer, BearerService, RequireAuthentication, Scoped,
};
pub use login::{
    CrossOrigin, InvalidMountPath, InvalidReturnTo, LoginLayer, LoginRoutes, LoginService,
    MaybeSignedIn, MountPath, PublicClient, ReturnTo, SignedIn, StepUp,
};
pub use seal::{CookieKey, CookieKeyError};
pub use session::StoredLogin;

use std::{future::Future, pin::Pin};

/// A boxed, sendable future, as the layers' services return.
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

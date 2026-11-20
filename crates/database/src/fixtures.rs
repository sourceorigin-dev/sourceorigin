//! What database tests start from: a connection, accounts, sessions and
//! delivery channels. This crate's tests use them, and so do other crates' tests,
//! behind the `test-util` feature.

use crate::{Database, DatabaseConfig, InsertSession, InsertUser, UserRepository};
use chrono::{TimeDelta, Utc};
use sourceorigin_common::{
    Handle, SessionId, UserId,
    events::{Browser, Platform, UserAgentFamily},
};
use std::net::IpAddr;

impl Database {
    /// Connects up to five connections to `TEST_DATABASE_URL`, with the time
    /// limits every connection of the service has, and applies the schema.
    ///
    /// # Panics
    ///
    /// Panics if `TEST_DATABASE_URL` is unset or invalid, or connecting or
    /// migrating fails.
    #[expect(
        clippy::expect_used,
        reason = "test-only fixture; documented under `# Panics`"
    )]
    pub async fn test() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let config = DatabaseConfig::new(&url, 5, std::time::Duration::from_secs(5))
            .expect("database configuration");
        let database = Self::connect(config).await.expect("connect");
        database.run_migrations().await.expect("migrate");
        database
    }
}

impl InsertUser {
    /// An account whose handle is `prefix` and a random suffix, for tests.
    ///
    /// # Panics
    ///
    /// Panics if `prefix` cannot start a handle.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "test-only constructor; documented under `# Panics`"
    )]
    pub fn named(prefix: &str) -> Self {
        let id = UserId::new();
        Self {
            id,
            handle: Handle::parse(&format!("{prefix}-{}", &id.to_string()[24..]))
                .expect("a handle prefix"),
            display_name: format!("{prefix} test"),
        }
    }
}

impl UserRepository<'_> {
    /// Creates the account [`InsertUser::named`] names after `prefix`, and
    /// returns its ID.
    ///
    /// # Panics
    ///
    /// Panics if `prefix` cannot start a handle, or the insert fails.
    #[expect(
        clippy::expect_used,
        reason = "test-only fixture; documented under `# Panics`"
    )]
    pub async fn insert_named(&mut self, prefix: &str) -> UserId {
        self.insert(&InsertUser::named(prefix))
            .await
            .expect("user")
            .id
    }
}

impl InsertSession {
    /// A new session of `user_id`, signed in now by no method from 192.0.2.1 in
    /// Firefox on Linux, unconfirmed, and ending after `lifetime`. Tests change the
    /// fields they test.
    #[must_use]
    pub fn lasting(user_id: UserId, lifetime: TimeDelta) -> Self {
        let now = Utc::now();
        Self {
            id: SessionId::new(),
            user_id,
            authentication_methods: Vec::new(),
            authenticated_at: now,
            expires_at: now + lifetime,
            user_agent_family: UserAgentFamily {
                browser: Browser::Firefox,
                platform: Platform::Linux,
            },
            ip: IpAddr::from([192, 0, 2, 1]),
            passkey_confirmed_at: None,
            passkey_id: None,
        }
    }
}

/// Declares channels whose deliveries tests insert themselves, so no event causes
/// one. Each entry reads `/// Doc comment` then
/// `Name = "stored_name": Recipient => Payload;`. Tests run at once on one
/// database, so each test that claims deliveries needs a stored name of its own.
/// The calling crate must depend on `sourceorigin-common`.
#[macro_export]
macro_rules! test_channels {
    ($($(#[doc = $doc:literal])* $name:ident = $value:literal: $recipient:ty => $payload:ty;)*) => {$(
        $(#[doc = $doc])*
        struct $name;

        impl $crate::Channel for $name {
            const NAME: &'static str = $value;
            type Recipient = $recipient;
            type Payload = $payload;

            /// Causes nothing: tests insert deliveries directly.
            fn deliveries(
                _: &::sourceorigin_common::events::DomainEvent,
                _: ::core::option::Option<::sourceorigin_common::UserId>,
                _: &[$crate::Notified],
                _: &mut $crate::DatabaseTransaction<'_>,
            ) -> impl ::core::future::Future<
                Output = ::core::result::Result<
                    ::std::vec::Vec<$crate::NewDelivery<Self>>,
                    $crate::DatabaseError,
                >,
            > + ::core::marker::Send {
                ::core::future::ready(::core::result::Result::Ok(::std::vec::Vec::new()))
            }
        }
    )*};
}

//! Domain repository handles backed by a private pool or transaction connection.
//!
//! This module holds only what every repository shares: the executor, the
//! declaration macro, the `run!` query macro, `query_record!`, which writes a
//! record's columns once, the `in_safe_order!` event order and the column types
//! that checked queries decode into. Each submodule owns one repository together with the
//! records it accepts and returns.

use crate::{Database, DatabaseError, DatabaseTransaction, ExclusiveConnection, LockKey};
use serde::de::DeserializeOwned;
use serde_json::Value;
use sourceorigin_common::events::DomainEvent;
use sqlx::{
    Decode, PgConnection, PgPool, Postgres, Transaction, Type,
    error::BoxDynError,
    postgres::{PgTypeInfo, PgValueRef},
    types::Json,
};

/// Where a repository runs its queries: directly on the pool, on the connection of
/// an open transaction, or on an [`ExclusiveConnection`]. Only a transaction keeps
/// locks past the statement that takes them, so [`Self::lock`] tells them apart.
pub(crate) enum RepositoryExecutor<'a> {
    /// Each operation borrows a connection from the shared pool.
    Pool(&'a PgPool),
    /// Every operation participates in one caller-owned transaction.
    Transaction(&'a mut PgConnection),
    /// Every operation runs on a locked [`ExclusiveConnection`], each statement in
    /// a transaction of its own.
    Exclusive(&'a mut PgConnection),
}

/// Declares each repository handle and its accessor on [`Database`],
/// [`DatabaseTransaction`] and [`ExclusiveConnection`], so all three always expose
/// the same families. Each entry reads `/// Doc comment` then `accessor: HandleName;`.
macro_rules! repositories {
    ($($(#[doc = $doc:literal])* $accessor:ident: $name:ident;)*) => {
        $(
            $(#[doc = $doc])*
            ///
            /// A focused persistence handle: it has no policy and no public SQL escape hatch.
            #[must_use]
            pub struct $name<'a> {
                /// Shared pool or caller-owned transaction used by query macros.
                executor: RepositoryExecutor<'a>,
            }

            impl<'a> $name<'a> {
                /// Binds this handle to the private shared pool.
                pub(crate) const fn from_pool(pool: &'a PgPool) -> Self {
                    Self { executor: RepositoryExecutor::Pool(pool) }
                }

                /// Binds this handle to a transaction for atomic operations.
                pub(crate) fn from_transaction(transaction: &'a mut Transaction<'_, Postgres>) -> Self {
                    Self { executor: RepositoryExecutor::Transaction(&mut **transaction) }
                }

                /// Binds this handle to a locked [`ExclusiveConnection`]'s connection.
                pub(crate) fn from_exclusive(connection: &'a mut PgConnection) -> Self {
                    Self { executor: RepositoryExecutor::Exclusive(connection) }
                }
            }
        )*

        impl Database {
            $(
                #[doc = concat!(
                    "Returns a [`", stringify!($name), "`] that runs each query on the pool."
                )]
                pub fn $accessor(&self) -> $name<'_> {
                    $name::from_pool(&self.pool)
                }
            )*
        }

        impl DatabaseTransaction<'_> {
            $(
                #[doc = concat!("Returns a [`", stringify!($name), "`] bound to this transaction.")]
                pub fn $accessor(&mut self) -> $name<'_> {
                    $name::from_transaction(&mut self.inner)
                }
            )*
        }

        impl ExclusiveConnection {
            $(
                #[doc = concat!(
                    "Returns a [`", stringify!($name), "`] bound to the locked connection."
                )]
                pub fn $accessor(&mut self) -> $name<'_> {
                    $name::from_exclusive(&mut self.connection)
                }
            )*
        }
    };
}

repositories! {
    /// User accounts.
    users: UserRepository;
    /// Current, released and retired handles.
    handles: HandleRepository;
    /// Verified email addresses and their cross-account ownership.
    emails: EmailRepository;
    /// Browser sessions, their cookie digests and passkey confirmations.
    sessions: SessionRepository;
    /// Typed domain events and their personal data.
    events: EventRepository;
    /// The audit chain over sealed events, its checkpoints and anchor.
    audit: AuditRepository;
    /// Outbound deliveries and their leases.
    deliveries: DeliveryRepository;
    /// Passkey credentials and `WebAuthn` ceremony state.
    passkeys: PasskeyRepository;
    /// OIDC clients, authorization requests, codes and consent.
    oidc: OidcRepository;
    /// Refresh token families and their redemption.
    tokens: TokenRepository;
    /// Social login transactions, pending signups and linked identities.
    federation: FederationRepository;
    /// Invitations and shared request rate limits.
    security: SecurityRepository;
    /// Deletion of expired ephemeral state, and counts of what is left of it.
    maintenance: MaintenanceRepository;
    /// Administrator rights.
    administrators: AdministratorRepository;
    /// Account recoveries and their links.
    recoveries: RecoveryRepository;
    /// Sign-ins to the admin dashboard: handoff codes and dashboard sessions.
    dashboard: DashboardRepository;
    /// Signing keys and their lifecycle.
    signing_keys: SigningKeyRepository;
    /// Accounts scheduled for deletion, and their purge.
    deletions: AccountDeletionRepository;
    /// Profile pictures.
    pictures: PictureRepository;
    /// The notifications of each account's inbox, and whether they were read.
    notifications: NotificationRepository;
    /// The alerts administrators are shown.
    alerts: AlertRepository;
    /// Announcements to accounts and their recipients.
    announcements: AnnouncementRepository;
    /// An account's records its data export reads in full, ended ones included.
    exports: ExportRepository;
}

/// How decoding some of a stored value's strings alone fails, which tells whether
/// this release knows the variants they name: a [`DomainEvent`], an [`Alert`], a
/// [`Notification`] or an [`Actor`] a newer release wrote reads as unknown, not as
/// corrupt. Only serde's derive knows the variant names, and it reports an unknown
/// one through [`serde::de::Error::unknown_variant`].
///
/// [`Alert`]: sourceorigin_common::alerts::Alert
/// [`Notification`]: sourceorigin_common::notifications::Notification
/// [`Actor`]: sourceorigin_common::events::Actor
#[derive(Debug, thiserror::Error)]
enum KindProbe {
    /// The kind is no variant of this release's enum.
    #[error("unknown kind")]
    Unknown,
    /// Anything else, such as the missing data of a known kind.
    #[error("{0}")]
    Other(String),
}

impl serde::de::Error for KindProbe {
    /// Keeps the message, for the error to be complete.
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self::Other(message.to_string())
    }

    /// Marks the kind as unknown.
    fn unknown_variant(_: &str, _: &'static [&'static str]) -> Self {
        Self::Unknown
    }
}

impl KindProbe {
    /// Whether this release's `T` knows every variant `fields` name, each a
    /// field of a stored `T` and its string value, such as its serde tag.
    fn knows<'a, T: DeserializeOwned>(fields: impl Iterator<Item = (&'a str, &'a str)>) -> bool {
        let fields = serde::de::value::MapDeserializer::<_, Self>::new(fields);
        !matches!(T::deserialize(fields), Err(Self::Unknown))
    }

    /// Decodes `tagged`, a stored `{"kind": …}` `T`, or returns `None` if this
    /// release does not know its kind.
    fn known<T: DeserializeOwned>(tagged: Value) -> Option<Result<T, serde_json::Error>> {
        let known = tagged
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| Self::knows::<T>(std::iter::once(("kind", kind))));
        known.then(|| serde_json::from_value(tagged))
    }

    /// Decodes `flat`, a stored `T` whose fields are its tag and values beside
    /// it, as an [`Actor`](sourceorigin_common::events::Actor)'s are, or returns
    /// `None` if one of its strings names a variant this release does not know:
    /// its tag, or a value such as the job of an `Actor::System`.
    fn known_flat<T: DeserializeOwned>(flat: &Value) -> Option<Result<T, serde_json::Error>> {
        let strings = (flat.as_object().into_iter().flatten())
            .filter_map(|(field, value)| Some((field.as_str(), value.as_str()?)));
        Self::knows::<T>(strings).then(|| T::deserialize(flat))
    }

    /// Decodes `tagged`, a `T` stored at payload `version`, if this release writes
    /// its kind in that version, which `version_of` tells; any other reads as
    /// `None`, as a newer release wrote it. A known kind at the first version, which
    /// every release reads, fails if it does not decode.
    fn versioned<T: DeserializeOwned>(
        tagged: Value,
        version: i16,
        version_of: impl Fn(&T) -> i16,
    ) -> Result<Option<T>, serde_json::Error> {
        match Self::known(tagged) {
            Some(Ok(value)) if version_of(&value) == version => Ok(Some(value)),
            Some(Err(source)) if version == DomainEvent::FIRST_VERSION => Err(source),
            Some(_) | None => Ok(None),
        }
    }
}

/// Runs a checked query on the repository's pool or transaction connection, in a
/// [`RepositoryExecutor::span`], and converts the driver error into a
/// [`DatabaseError`](crate::DatabaseError), e.g.
/// `run!(self, fetch_optional, sqlx::query_as!(StoredUser, "...", id.as_uuid()))`.
///
/// `$method` is the `sqlx` executor method: `execute`, `fetch_one` (a missing row
/// becomes [`DatabaseError::NotFound`](crate::DatabaseError::NotFound)), `fetch_optional` or `fetch_all`. Two
/// more `execute` a statement and report what it changed: `changed_one` returns
/// whether it changed exactly one row, and `required` returns `()`, or
/// [`DatabaseError::NotFound`](crate::DatabaseError::NotFound) if it changed none. A macro
/// rather than a method because `&PgPool` and `&mut PgConnection` are different
/// executor types. `run!(@on executor, ...)` runs on a [`RepositoryExecutor`] itself.
macro_rules! run {
    (@on $executor:expr, $method:ident, $query:expr) => {{
        let query = $query;
        let span = RepositoryExecutor::span(sqlx::Execute::sql(&query));
        let result = tracing::Instrument::instrument(
            async {
                match &mut $executor {
                    RepositoryExecutor::Pool(pool) => query.$method(*pool).await,
                    RepositoryExecutor::Transaction(connection)
                    | RepositoryExecutor::Exclusive(connection) => {
                        query.$method(&mut **connection).await
                    }
                }
            },
            span.clone(),
        )
        .await
        .map_err(Into::<DatabaseError>::into);
        RepositoryExecutor::finish(&span, result)
    }};
    ($self:expr, changed_one, $query:expr) => {
        run!($self, execute, $query).map(|done| done.rows_affected() == 1)
    };
    ($self:expr, required, $query:expr) => {
        run!($self, execute, $query).and_then(|done| match done.rows_affected() {
            0 => Err(crate::DatabaseError::NotFound),
            _ => Ok(()),
        })
    };
    ($self:expr, $method:ident, $query:expr) => {
        run!(@on $self.executor, $method, $query)
    };
}

/// A checked `sqlx::query_as!` returning `$record`s, with their `$columns`
/// between `$head`, which ends where they go, after `SELECT` or `RETURNING`, and
/// `$tail`, literals joined with `+`. A record that several queries read declares
/// a macro over this one, so its columns are written once, e.g.
/// `passkeys!("SELECT", "FROM passkey_credentials p WHERE p.id = $1", id)`.
macro_rules! query_record {
    (
        $record:path,
        $columns:literal,
        $head:literal,
        $tail:literal $(+ $more:literal)*
        $(, $arg:expr)* $(,)?
    ) => {
        sqlx::query_as!($record, $head + " " + $columns + " " + $tail $(+ $more)* $(, $arg)*)
    };
}

/// A checked `sqlx::query_as!` over `domain_events e` in the safe order, the rule
/// every reader that follows the stream, such as the audit sealer, keeps to:
/// `$query` selects and filters, the macro adds the rule, and `$tail` follows it
/// (`LIMIT`, row locks), e.g.
/// `in_safe_order!(Row, "SELECT e.id FROM domain_events e WHERE true", "LIMIT $1", 10)`.
///
/// Sequence order alone is not safe: a transaction can take sequence 10 and commit
/// after another took 11, so a reader that saw 11 would skip 10. Events are read in
/// `(txid, sequence)` order instead, and only from transactions older than every
/// transaction still running. Those have all finished, so no event can later appear
/// before one already read. The cost is that a long-running transaction holds back
/// every reader until it ends.
macro_rules! in_safe_order {
    ($record:ty, $query:literal, $tail:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            $record,
            $query
                + " AND e.txid < pg_snapshot_xmin(pg_current_snapshot())"
                + " ORDER BY e.txid, e.sequence "
                + $tail
            $(, $arg)*
        )
    };
}

impl RepositoryExecutor<'_> {
    /// Waits for `key`'s transaction-scoped advisory lock, held until the
    /// transaction ends.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::OutsideTransaction`] outside a transaction, where
    /// the lock would end with its statement, or another [`DatabaseError`] if the
    /// query fails.
    async fn lock(&mut self, key: LockKey) -> Result<(), DatabaseError> {
        self.require_transaction()?;
        run!(@on *self, execute, sqlx::query!("SELECT pg_advisory_xact_lock($1)", key.id()))?;
        Ok(())
    }

    /// Takes the lock of [`Self::lock`] if no other transaction holds it, and
    /// returns whether it did.
    ///
    /// # Errors
    ///
    /// As [`Self::lock`].
    async fn try_lock(&mut self, key: LockKey) -> Result<bool, DatabaseError> {
        self.require_transaction()?;
        run!(
            @on *self,
            fetch_one,
            sqlx::query_scalar!(r#"SELECT pg_try_advisory_xact_lock($1) AS "locked!""#, key.id())
        )
    }

    /// Refuses work whose locks must last until its transaction ends, such as a
    /// transaction-scoped advisory lock or `FOR UPDATE`, outside a transaction,
    /// where they would end with the statement that takes them.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::OutsideTransaction`] on the pool or an
    /// [`ExclusiveConnection`].
    fn require_transaction(&self) -> Result<(), DatabaseError> {
        match self {
            Self::Transaction(_) => Ok(()),
            Self::Pool(_) | Self::Exclusive(_) => Err(DatabaseError::OutsideTransaction),
        }
    }

    /// A client span for one query, recording its parameterized `sql` as
    /// `db.query.text`, never its bind values. A query outside any trace, such as an
    /// idle delivery worker's poll, gets none: each would be a trace of its own.
    fn span(sql: &str) -> tracing::Span {
        if tracing::Span::current().is_none() {
            return tracing::Span::none();
        }
        let operation = sql.split_whitespace().next().unwrap_or("query");
        tracing::info_span!(
            "db.query",
            otel.name = operation,
            otel.kind = "client",
            otel.status_code = tracing::field::Empty,
            db.system.name = "postgresql",
            db.query.text = sql,
        )
    }

    /// Marks `span` failed when the query failed for a reason other than a
    /// missing row or a conflict, which callers expect.
    fn finish<T>(
        span: &tracing::Span,
        result: Result<T, DatabaseError>,
    ) -> Result<T, DatabaseError> {
        if let Err(
            DatabaseError::Refused(_)
            | DatabaseError::Timeout(_)
            | DatabaseError::Cancelled(_)
            | DatabaseError::LockUnavailable(_)
            | DatabaseError::InvalidStoredData { .. }
            | DatabaseError::Operation(_),
        ) = &result
        {
            span.record("otel.status_code", "ERROR");
        }
        result
    }
}

/// A JSONB array column decoded straight into a record's `Vec` field, e.g.
/// `scopes AS "scopes: JsonArray<String>"` in `query_as!`, which converts each
/// overridden column with `Into`. Invalid JSON fails decoding.
#[derive(Debug)]
struct JsonArray<T>(Vec<T>);

impl<T> Type<Postgres> for JsonArray<T> {
    /// Stored as `JSONB`.
    fn type_info() -> PgTypeInfo {
        Json::<Vec<T>>::type_info()
    }
}

impl<'r, T: DeserializeOwned + 'r> Decode<'r, Postgres> for JsonArray<T> {
    /// Deserializes the stored array.
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        Json::<Vec<T>>::decode(value).map(|Json(values)| Self(values))
    }
}

impl<T> From<JsonArray<T>> for Vec<T> {
    /// Unwraps the decoded array.
    fn from(JsonArray(values): JsonArray<T>) -> Self {
        values
    }
}

/// A `BIGINT` column that must not be negative, decoded into a `u64` field, e.g.
/// `profile_version AS "profile_version: Unsigned"`.
#[derive(Debug)]
struct Unsigned(u64);

impl Type<Postgres> for Unsigned {
    /// Stored as `BIGINT`.
    fn type_info() -> PgTypeInfo {
        i64::type_info()
    }
}

impl Decode<'_, Postgres> for Unsigned {
    /// Rejects negative values.
    fn decode(value: PgValueRef<'_>) -> Result<Self, BoxDynError> {
        Ok(Self(u64::try_from(i64::decode(value)?)?))
    }
}

impl Unsigned {
    /// Converts a value for a `BIGINT` bind.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::Operation`](DatabaseError::Operation) for a
    /// value above `i64::MAX`, which no stored value reaches.
    fn bind(value: u64) -> Result<i64, DatabaseError> {
        i64::try_from(value).map_err(|error| DatabaseError::Operation(Box::new(error)))
    }
}

impl From<Unsigned> for u64 {
    /// Unwraps the decoded value.
    fn from(Unsigned(value): Unsigned) -> Self {
        value
    }
}

/// A `BYTEA` column holding exactly 32 bytes, such as a SHA-256 hash, decoded
/// into a `[u8; 32]` field, e.g. `head AS "head: Bytes32"`.
#[derive(Debug)]
struct Bytes32([u8; 32]);

impl Type<Postgres> for Bytes32 {
    /// Stored as `BYTEA`.
    fn type_info() -> PgTypeInfo {
        Vec::<u8>::type_info()
    }
}

impl Decode<'_, Postgres> for Bytes32 {
    /// Rejects any other length.
    fn decode(value: PgValueRef<'_>) -> Result<Self, BoxDynError> {
        let bytes = <&[u8]>::decode(value)?;
        Ok(Self(bytes.try_into()?))
    }
}

impl From<Bytes32> for [u8; 32] {
    /// Unwraps the decoded bytes.
    fn from(Bytes32(value): Bytes32) -> Self {
        value
    }
}

/// Administrator rights and the lock that serializes their changes.
pub(crate) mod administrators;
/// The alerts administrators are shown.
pub(crate) mod alerts;
/// Announcements, their drafts and their recipients.
pub(crate) mod announcements;
/// The audit chain, its checkpoints and its anchor.
pub(crate) mod audit;
/// Handoff codes and dashboard sessions.
pub(crate) mod dashboard;
/// Accounts scheduled for deletion, restore tokens and the purge.
pub(crate) mod deletions;
/// The outbound delivery queue.
pub(crate) mod deliveries;
/// Verified email ownership and primary-address persistence.
pub(crate) mod emails;
/// Typed domain events and their personal data.
pub(crate) mod events;
/// An account's records its data export reads in full.
pub(crate) mod exports;
/// Provider transactions, pending signups and external identity links.
pub(crate) mod federation;
/// Every handle, current or released, and the rename that moves between them.
pub(crate) mod handles;
/// Expired-state cleanup.
pub(crate) mod maintenance;
/// Each account's inbox.
pub(crate) mod notifications;
/// Client registrations, authorization requests, codes and consent.
pub(crate) mod oidc;
/// Passkey credentials and single-use ceremony records.
pub(crate) mod passkeys;
/// Profile pictures and what their URLs serve.
pub(crate) mod pictures;
/// Account recoveries and their links.
pub(crate) mod recoveries;
/// Signup invitations and shared request budgets.
pub(crate) mod security;
/// Browser sessions, cookie digests and passkey confirmations.
pub(crate) mod sessions;
/// Signing keys, their lifecycle and its lock.
pub(crate) mod signing_keys;
/// Focused tests for the column types and the lock guard shared by every repository.
#[cfg(test)]
mod tests;
/// Refresh-token grants, their redemption and replay revocation.
pub(crate) mod tokens;
/// Account records and serialization locks.
pub(crate) mod users;

//! Analytics: typed rows, each a [`Signal`](crate::analytics::Signal), stored in
//! ClickHouse.
//!
//! A signal is a struct declared with [`signal!`](crate::signal), which is also
//! its table, which `auth-service migrate` creates and extends
//! ([`Store::migrate`](crate::analytics::Store::migrate)). Every field's type
//! must be a [`Column`](crate::analytics::Column): numbers, booleans,
//! `&'static str` vocabularies and pseudonymous IDs, so a name, an address or a
//! path does not compile. [`Signal::record`](crate::analytics::Signal::record)
//! queues a row; a writer sends each table's in batches and drops what it can't
//! send. See `docs/dev/architecture/analytics.md`.

/// The ClickHouse client and its configuration.
mod store;
/// The queue and the task that sends it.
mod writer;

/// Rows recorded on this thread, captured for tests.
#[cfg(any(test, feature = "test-util"))]
mod capture;

#[cfg(any(test, feature = "test-util"))]
pub use capture::AnalyticsCapture;
pub use store::{Migration, OWNER_VARIABLES, READER_VARIABLES, Store, StoreError, VARIABLES};
pub use writer::PRODUCT_INTERVAL;
pub(crate) use writer::Writer;

use serde::{Serialize, Serializer};
use sourceorigin_common::{ClientId, EventId, TraceId};
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

/// One kind of row, about how SourceOrigin runs, and the table that stores it.
/// Declare one with [`signal!`](crate::signal), which implements this trait.
pub trait Signal: Serialize {
    /// The table, as `migrate` creates it.
    const TABLE: Table;

    /// Queues this row, stamped with the current time, for its table. It never
    /// waits: a full queue drops the row, and counts it.
    fn record(&self) {
        writer::record(Self::TABLE.name, writer::Stamp::Millisecond, self);
    }
}

/// One kind of row about how people use SourceOrigin, which a person may keep
/// out of: recording one takes their [`Consent`]. Declare one with
/// `signal!`'s `product struct`.
pub trait Product: Serialize {
    /// The table, as `migrate` creates it.
    const TABLE: Table;

    /// Queues this row, as [`Signal::record`] does, with `consent` to show the
    /// person it is about takes part in usage statistics. Its time holds the
    /// day only, and it is sent with its hour's others, so neither points to the
    /// moment of the sign-in or change it counts, unless it is its hour's only
    /// one.
    fn record(&self, consent: Consent) {
        let Consent(()) = consent;
        writer::record(Self::TABLE.name, writer::Stamp::Day, self);
    }
}

/// That the person a [`Product`] row is about takes part in usage statistics:
/// their account did not leave them, and their browser asks no
/// [Global Privacy Control](https://globalprivacycontrol.org).
#[derive(Clone, Copy, Debug)]
pub struct Consent(());

impl Consent {
    /// The `Sec-GPC` header, `1` when the browser asks not to be tracked.
    pub const GPC_HEADER: &str = "sec-gpc";

    /// Consent, unless the account left usage statistics (`included` is false)
    /// or the request carried `Sec-GPC: 1` (`gpc`). For a person who isn't
    /// signed in, `included` is true.
    #[must_use]
    pub fn given(included: bool, gpc: bool) -> Option<Self> {
        (included && !gpc).then_some(Self(()))
    }
}

/// A table's declaration, from its signal's.
#[derive(Debug)]
pub struct Table {
    /// Its name, such as `events`.
    pub name: &'static str,
    /// What one row records, from the signal's doc comment.
    pub description: &'static [&'static str],
    /// Days a row is kept, counted from its time.
    pub retention_days: u16,
    /// The columns rows are sorted by, before their time: what queries filter on
    /// most, fewest distinct values first.
    pub order_by: &'static [&'static str],
    /// Every column but `time`, which every table starts with.
    pub columns: &'static [ColumnDeclaration],
}

/// One column of a [`Table`].
#[derive(Debug)]
pub struct ColumnDeclaration {
    /// Its name, the field's.
    pub name: &'static str,
    /// Its ClickHouse type, the field type's [`Column::TYPE`].
    pub ty: &'static str,
    /// What it holds, from the field's doc comment.
    pub description: &'static [&'static str],
}

/// A Rust type a signal's field may have, and the ClickHouse type that stores
/// it. Only types that hold no free text and no directly identifying data
/// implement it: a closed vocabulary is a `&'static str`, so no string a user
/// or a client typed can become one, and an account's [`UserId`](sourceorigin_common::UserId)
/// is not one, since an account's export and purge don't reach ClickHouse. A
/// `ClientId` must be a registered client's. It is sealed: only this module
/// implements it.
pub trait Column: sealed::Sealed {
    /// The ClickHouse type, e.g. `LowCardinality(String)`.
    const TYPE: &'static str;
}

/// Keeps [`Column`] to the types this module names.
mod sealed {
    /// Implemented by every [`Column`](super::Column), here only.
    pub trait Sealed {}
}

/// Implements [`Column`] for each `type => "ClickHouse type"` pair.
/// A type followed by `key` is also a [`Key`].
macro_rules! columns {
    ($($ty:ty => $sql:literal $($key:ident)?;)+) => {
        $(
            impl sealed::Sealed for $ty {}
            impl Column for $ty {
                const TYPE: &'static str = $sql;
            }
            $(columns!(@$key $ty);)?
        )+
    };
    (@key $ty:ty) => {
        impl Key for $ty {}
    };
}

/// A [`Column`] rows may be sorted by: any but a nullable one, which ClickHouse
/// refuses as a sort key.
#[diagnostic::on_unimplemented(
    message = "`{Self}` can't be in `ordered by`: ClickHouse refuses a nullable sort key"
)]
pub trait Key: Column {}

columns! {
    bool => "Bool" key;
    u8 => "UInt8" key;
    u16 => "UInt16" key;
    u32 => "UInt32" key;
    Option<u32> => "Nullable(UInt32)";
    u64 => "UInt64" key;
    i64 => "Int64" key;
    f64 => "Float64" key;
    &'static str => "LowCardinality(String)" key;
    Option<&'static str> => "LowCardinality(Nullable(String))";
    ClientId => "LowCardinality(String)" key;
    Option<ClientId> => "LowCardinality(Nullable(String))";
    EventId => "UUID" key;
    Option<TraceId> => "Nullable(FixedString(32))";
    Option<Route> => "LowCardinality(Nullable(String))";
    Instance => "LowCardinality(String)" key;
}

/// This process: a new UUID each time it starts, which its spans and logs
/// carry as `service.instance.id` too, unless `OTEL_RESOURCE_ATTRIBUTES` names
/// another, so the replicas of a service, and its command-line runs, tell their
/// rows apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance(Arc<str>);

impl Instance {
    /// This process's.
    #[must_use]
    pub fn current() -> Self {
        /// Chosen on first use.
        static CURRENT: OnceLock<Instance> = OnceLock::new();
        CURRENT
            .get_or_init(|| Self(Arc::from(Uuid::now_v7().to_string())))
            .clone()
    }

    /// The UUID, as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for Instance {
    /// Writes the UUID.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// A route template of a router, such as `/account/passkeys/{id}`: a column only
/// [`HttpTelemetry`](crate::HttpTelemetry) builds, from the router's own table,
/// so it is never a path a client sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route(Arc<str>);

impl Route {
    /// The template a request matched.
    pub(crate) fn matched(path: &axum::extract::MatchedPath) -> Self {
        Self(Arc::from(path.as_str()))
    }

    /// The template, as written in the router.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for Route {
    /// Writes the template.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// Declares a [`Signal`]: a struct whose fields are [`Column`]s, and the table
/// that stores it, with how long rows are kept and the columns they are sorted
/// by, each a [`Key`]. Every table also has a `time` column, the time the row
/// was recorded, to the day for a product signal, and sorts by it last. Doc
/// comments are required: they document the table.
///
/// ```
/// sourceorigin_telemetry::signal! {
///     /// A request refused by a rate limit.
///     pub struct Throttled in "throttled", kept 30 days, ordered by (category) {
///         /// Its rate category, from a closed vocabulary.
///         pub category: &'static str,
///     }
/// }
///
/// use sourceorigin_telemetry::analytics::Signal;
/// assert_eq!(Throttled::TABLE.columns[0].ty, "LowCardinality(String)");
/// Throttled { category: "login" }.record();
/// ```
///
/// A product signal, about how people use SourceOrigin, is a `product struct`:
/// its `record` takes a [`Consent`], and it has no other.
///
/// ```compile_fail
/// sourceorigin_telemetry::signal! {
///     /// A passkey someone signed in with.
///     pub product struct PasskeyUsed in "passkeys_used", kept 390 days, ordered by (synced) {
///         /// Whether it is synced.
///         pub synced: bool,
///     }
/// }
///
/// use sourceorigin_telemetry::analytics::Signal;
/// PasskeyUsed { synced: true }.record();
/// ```
///
/// A nullable sort key does not compile:
///
/// ```compile_fail
/// sourceorigin_telemetry::signal! {
///     /// A request refused by a rate limit.
///     pub struct Throttled in "throttled", kept 30 days, ordered by (category) {
///         /// Its rate category, if it has one.
///         pub category: Option<&'static str>,
///     }
/// }
/// ```
///
/// Nor does a field that could hold free text, such as a `String`:
///
/// ```compile_fail
/// sourceorigin_telemetry::signal! {
///     /// A search.
///     pub struct Searched in "searches", kept 30 days, ordered by (query) {
///         /// What was searched for.
///         pub query: String,
///     }
/// }
/// ```
#[macro_export]
macro_rules! signal {
    (
        $(#[doc = $doc:literal])+
        $vis:vis product struct $($rest:tt)*
    ) => {
        $crate::signal!(@declare $crate::analytics::Product, [$($doc)+] $vis $($rest)*);
    };
    (
        $(#[doc = $doc:literal])+
        $vis:vis struct $($rest:tt)*
    ) => {
        $crate::signal!(@declare $crate::analytics::Signal, [$($doc)+] $vis $($rest)*);
    };
    (
        @declare $trait:path, [$($doc:literal)+] $vis:vis $name:ident in $table:literal,
            kept $days:literal days, ordered by ($($key:ident),* $(,)?) {
            $($(#[doc = $field_doc:literal])+ $field_vis:vis $field:ident: $ty:ty,)+
        }
    ) => {
        $(#[doc = $doc])+
        #[derive(Debug, Clone, $crate::__serde::Serialize)]
        #[serde(crate = "sourceorigin_telemetry::__serde")]
        $vis struct $name {
            $($(#[doc = $field_doc])+ $field_vis $field: $ty,)+
        }

        impl $trait for $name {
            const TABLE: $crate::analytics::Table = $crate::analytics::Table {
                name: $table,
                description: &[$($doc),+],
                retention_days: $days,
                order_by: &[$({
                    const _: fn(&$name) = |row: &$name| {
                        fn key<T: $crate::analytics::Key>(_: &T) {}
                        key(&row.$key);
                    };
                    stringify!($key)
                }),*],
                columns: &[$($crate::analytics::ColumnDeclaration {
                    name: stringify!($field),
                    ty: <$ty as $crate::analytics::Column>::TYPE,
                    description: &[$($field_doc),+],
                }),+],
            };
        }
    };
}

#[cfg(test)]
mod tests;

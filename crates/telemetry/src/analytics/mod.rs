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
pub use store::{OWNER_VARIABLES, Store, StoreError, VARIABLES};
pub(crate) use writer::Writer;

use serde::{Serialize, Serializer};
use sourceorigin_common::{ClientId, EventId, TraceId};
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

/// One kind of row, and the table that stores it. Declare one with
/// [`signal!`](crate::signal), which implements this trait.
pub trait Signal: Serialize {
    /// The table, as `migrate` creates it.
    const TABLE: Table;

    /// Queues this row, stamped with the current time, for its table. It never
    /// waits: a full queue drops the row, and counts it.
    fn record(&self) {
        writer::record(Self::TABLE.name, self);
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
    /// How its rows are kept.
    pub kind: TableKind,
}

/// How a [`Table`]'s rows are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableKind {
    /// A signal's: partitioned by month, each row's time to the millisecond.
    Signal,
    /// Spans' or log lines': partitioned by day, each row's time to the
    /// microsecond, with a bloom filter on `trace`, so a lookup by trace reads
    /// little, and a day's part dropped whole once every row in it expired.
    Telemetry,
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
/// was recorded, and sorts by it last. Doc comments are required: they document
/// the table.
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
        $vis:vis struct $name:ident in $table:literal,
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

        impl $crate::analytics::Signal for $name {
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
                kind: $crate::analytics::TableKind::Signal,
            };
        }
    };
}

#[cfg(test)]
mod tests;

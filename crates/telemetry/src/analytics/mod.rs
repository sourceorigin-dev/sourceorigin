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

use serde::Serialize;
use sourceorigin_common::{ClientId, EventId, TraceId};

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
macro_rules! columns {
    ($($ty:ty => $sql:literal,)+) => {
        $(
            impl sealed::Sealed for $ty {}
            impl Column for $ty {
                const TYPE: &'static str = $sql;
            }
        )+
    };
}

columns! {
    bool => "Bool",
    u8 => "UInt8",
    u16 => "UInt16",
    u32 => "UInt32",
    u64 => "UInt64",
    i64 => "Int64",
    f64 => "Float64",
    &'static str => "LowCardinality(String)",
    Option<&'static str> => "LowCardinality(Nullable(String))",
    ClientId => "LowCardinality(String)",
    Option<ClientId> => "LowCardinality(Nullable(String))",
    EventId => "UUID",
    Option<TraceId> => "Nullable(FixedString(32))",
}

/// Declares a [`Signal`]: a struct whose fields are [`Column`]s, and the table
/// that stores it, with how long rows are kept and the columns they are sorted
/// by. Every table also has a `time` column, the time the row was recorded, and
/// sorts by it last. Doc comments are required: they document the table.
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
/// A field that could hold free text, such as a `String`, does not compile:
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
                    // Fails to compile unless the key is one of the fields.
                    let _ = |row: &$name| { let _ = &row.$key; };
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

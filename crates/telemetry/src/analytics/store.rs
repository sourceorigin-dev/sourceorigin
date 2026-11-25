//! ClickHouse through its own client, `clickhouse`, over its HTTP interface:
//! inserts as `JSONEachRow`, the statements `migrate` runs, and typed reads.

use super::{Table, TableKind};
use bytes::Bytes;
use secrecy::{ExposeSecret, SecretString};
use sourceorigin_common::config::{ConfigurationError, SecretReference, Variable};
use std::{fmt::Write as _, time::Duration};
use thiserror::Error;
use url::Url;

sourceorigin_common::variables! {
    /// The variables of every service that records analytics, in the
    /// configuration reference's order.
    pub const VARIABLES;

    /// The ClickHouse database analytics rows are written to, as an HTTP URL
    /// naming the user and the database, such as
    /// `http://sourceorigin_writer@clickhouse:8123/sourceorigin`. Unset or empty,
    /// rows are discarded.
    const URL = "ANALYTICS_URL";
    /// Where to read that user's password, through `env://NAME` or
    /// `file:///path`.
    const PASSWORD = "ANALYTICS_PASSWORD";
}

sourceorigin_common::variables! {
    /// The variables of the service that reads analytics back, auth, for its
    /// admin dashboard and each app's Insights, in the configuration
    /// reference's order.
    pub const READER_VARIABLES;

    /// The ClickHouse user auth reads analytics back as, for the admin
    /// dashboard's Analytics page and each app's Insights, in the same format as
    /// `ANALYTICS_URL`, such as
    /// `http://sourceorigin_reader@clickhouse:8123/sourceorigin`. With
    /// `ANALYTICS_OWNER_URL`, `migrate` creates or resets it, allowed only to
    /// read the signals' tables, and every table's name and size, within limits
    /// on each query, so it must be a user of its own. Unset, the Analytics page
    /// says analytics are off, and Insights that they aren't available.
    const READER_URL = "ANALYTICS_READER_URL";
    /// Where to read that user's password, as for `ANALYTICS_PASSWORD`.
    const READER_PASSWORD = "ANALYTICS_READER_PASSWORD";
}

sourceorigin_common::variables! {
    /// The variables `migrate` reads too, in the configuration reference's order.
    pub const OWNER_VARIABLES;

    /// The ClickHouse user `migrate` connects as instead, in the same format as
    /// `ANALYTICS_URL`. `migrate` then creates or resets the user
    /// `ANALYTICS_URL` names, with the `ANALYTICS_PASSWORD` password, allowed
    /// only to insert rows. The owner must be allowed to manage users.
    const OWNER_URL = "ANALYTICS_OWNER_URL";
    /// Where to read the owner's password, as for `ANALYTICS_PASSWORD`.
    const OWNER_PASSWORD = "ANALYTICS_OWNER_PASSWORD";
}

/// How long one request to ClickHouse may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// What each of the reader's queries may use, which ClickHouse enforces: no
/// change, 5 seconds, 512 MiB, and 10,000 rows back; and 16 queries at once,
/// so however many pages read, they can't take ClickHouse over. Its rows'
/// 64-bit integers are JSON numbers, never quoted, whatever the server's
/// default.
const READER_LIMITS: &str = "readonly = 1, max_execution_time = 5, \
                             max_memory_usage = 536870912, max_result_rows = 10000, \
                             max_concurrent_queries_for_user = 16, \
                             output_format_json_quote_64bit_integers = 0";
/// What the reader may read of ClickHouse's own tables: how much each table
/// stores, for the dashboard's storage card. `system.parts` lists only the
/// tables a user may see, so the reader may see every table's name, though
/// not read `spans` and `logs`.
const READER_SYSTEM: &str = "SELECT(database, table, active, rows, bytes_on_disk) ON system.parts";

/// What `migrate` connects as, and the users it provisions.
pub struct Migration {
    /// The owner, or the writer when no owner is configured.
    pub store: Store,
    /// The writer, when connected as the owner.
    pub writer: Option<Store>,
    /// The reader, when connected as the owner and it is configured.
    pub reader: Option<Store>,
}

/// A ClickHouse user's connection to one database.
#[derive(Clone)]
pub struct Store {
    /// Sends the requests, as the user, to the database.
    client: clickhouse::Client,
    /// The user, a plain identifier.
    user: String,
    /// The user's password, which `migrate` sets when it provisions the user.
    password: SecretString,
    /// The database, a plain identifier.
    database: String,
}

/// Why ClickHouse refused or never answered a request. It never holds what
/// ClickHouse answered an insert with, which can quote the rows.
#[derive(Debug, Error)]
pub enum StoreError {
    /// ClickHouse did not answer within the time one request may take.
    #[error("ClickHouse did not answer within {} seconds", REQUEST_TIMEOUT.as_secs())]
    TimedOut(#[from] tokio::time::error::Elapsed),
    /// The request did not complete, such as when ClickHouse is unreachable.
    #[error("could not reach ClickHouse")]
    Unreachable(#[source] clickhouse::error::Error),
    /// ClickHouse refused an insert, with its error code.
    #[error("ClickHouse refused the insert into {table} (code {code})")]
    Insert {
        /// The table.
        table: &'static str,
        /// ClickHouse's error code, such as `60` for a missing table, or
        /// `unknown`.
        code: String,
    },
    /// ClickHouse refused a query of auth's own, or its rows do not match the
    /// row type it is read as.
    #[error("ClickHouse refused `{query}`, or its rows don't match their type")]
    Query {
        /// The query.
        query: &'static str,
        /// What ClickHouse answered, or which column did not match.
        source: clickhouse::error::Error,
    },
    /// ClickHouse refused a statement of `migrate`.
    #[error("ClickHouse refused `{statement}`")]
    Statement {
        /// The statement, without any password it set.
        statement: String,
        /// What ClickHouse answered.
        source: clickhouse::error::Error,
    },
}

impl Store {
    /// The connection `ANALYTICS_URL` and `ANALYTICS_PASSWORD` configure, if any.
    ///
    /// # Errors
    /// Fails when either is malformed, or the password cannot be read.
    pub fn from_environment() -> Result<Option<Self>, ConfigurationError> {
        Self::read(URL, PASSWORD)
    }

    /// What `migrate` connects with: the owner, with the writer it provisions,
    /// when `ANALYTICS_OWNER_URL` is set; otherwise the writer alone, which then
    /// needs the rights to create the database and its tables itself. `None`
    /// without `ANALYTICS_URL`.
    ///
    /// # Errors
    /// Fails when a variable is malformed, a password cannot be read, the
    /// owner is set without the writer's password, or the reader is the owner
    /// or the writer, which `migrate` would lock out or stop inserting.
    pub fn migration_from_environment() -> Result<Option<Migration>, ConfigurationError> {
        let Some(writer) = Self::from_environment()? else {
            return Ok(None);
        };
        Ok(Some(match Self::read(OWNER_URL, OWNER_PASSWORD)? {
            Some(owner) => {
                let reader = Self::reader_from_environment()?;
                if reader
                    .as_ref()
                    .is_some_and(|reader| [&owner.user, &writer.user].contains(&&reader.user))
                {
                    return Err(READER_URL.invalid(
                        "must name a user of its own, not ANALYTICS_URL's or ANALYTICS_OWNER_URL's",
                    ));
                }
                Migration {
                    store: owner,
                    writer: Some(writer),
                    reader,
                }
            }
            None => Migration {
                store: writer,
                writer: None,
                reader: None,
            },
        }))
    }

    /// The connection `ANALYTICS_READER_URL` and `ANALYTICS_READER_PASSWORD`
    /// configure, if any: auth's, for the admin dashboard and each app's
    /// Insights.
    ///
    /// # Errors
    /// Fails when either is malformed, or the password cannot be read.
    pub fn reader_from_environment() -> Result<Option<Self>, ConfigurationError> {
        Self::read(READER_URL, READER_PASSWORD)
    }

    /// Reads one connection from its URL and password variables.
    fn read(url: Variable, password: Variable) -> Result<Option<Self>, ConfigurationError> {
        let Some(value) = url.optional()?.filter(|value| !value.trim().is_empty()) else {
            return Ok(None);
        };
        let reference: SecretReference = password.parse()?;
        let secret = reference
            .resolve()
            .map_err(|source| password.error("must name a readable secret", source))?;
        Self::new(url, &value, SecretString::from(secret.text()?)).map(Some)
    }

    /// Connects to `value`, the URL `variable` holds, with `password`.
    pub(crate) fn new(
        variable: Variable,
        value: &str,
        password: SecretString,
    ) -> Result<Self, ConfigurationError> {
        const SHAPE: &str = "must be an http:// URL naming a user and a database, such as \
                             http://USER@HOST:8123/DATABASE, without a password: ClickHouse \
                             is only reached inside the deployment's network";
        let mut endpoint = Url::parse(value).map_err(|source| variable.error(SHAPE, source))?;
        let user = endpoint.username().to_owned();
        let database = endpoint.path().trim_matches('/').to_owned();
        if endpoint.scheme() != "http"
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || !Self::identifier(&user)
            || !Self::identifier(&database)
        {
            return Err(variable.invalid(SHAPE));
        }
        endpoint.set_path("/");
        endpoint.set_username("").ok();
        Ok(Self::connect(&endpoint, user, password, database))
    }

    /// The client of `user` with `password`, to `database` at `endpoint`.
    fn connect(endpoint: &Url, user: String, password: SecretString, database: String) -> Self {
        let client = clickhouse::Client::default()
            .with_url(endpoint.as_str())
            .with_database(&database);
        Self::signed_in(client, user, password, database)
    }

    /// `client`, which names the server and the database, as `user` with
    /// `password`.
    fn signed_in(
        client: clickhouse::Client,
        user: String,
        password: SecretString,
        database: String,
    ) -> Self {
        Self {
            client: client
                .with_user(&user)
                .with_password(password.expose_secret()),
            user,
            password,
            database,
        }
    }

    /// The same server as another user, e.g. the writer `migrate` provisions.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn as_user(&self, user: &str, password: &str) -> Self {
        Self::signed_in(
            self.client.clone(),
            user.to_owned(),
            SecretString::from(password),
            self.database.clone(),
        )
    }

    /// Inserts `rows`, one JSON object per line, into `table`, as a test seeds
    /// a table.
    ///
    /// # Errors
    /// Returns what ClickHouse refused the insert with.
    #[cfg(any(test, feature = "test-util"))]
    pub async fn insert_rows(&self, table: &'static str, rows: String) -> Result<(), StoreError> {
        self.insert(table, Bytes::from(rows)).await
    }

    /// The first column of `query`'s rows, as text, one row per line.
    ///
    /// # Errors
    /// Returns what ClickHouse refused the query with.
    #[cfg(any(test, feature = "test-util"))]
    pub async fn select(&self, query: &str) -> Result<String, StoreError> {
        let refused = |source| StoreError::Statement {
            statement: query.to_owned(),
            source,
        };
        let mut cursor = self
            .client
            .query_raw(query)
            .fetch_bytes("TSV")
            .map_err(refused)?;
        let text = Self::within(cursor.collect()).await?.map_err(refused)?;
        Ok(String::from_utf8_lossy(&text).trim_end().to_owned())
    }

    /// The result of `request`, unless ClickHouse takes longer than
    /// [`REQUEST_TIMEOUT`] to answer it.
    async fn within<T>(
        request: impl Future<Output = clickhouse::error::Result<T>>,
    ) -> Result<clickhouse::error::Result<T>, StoreError> {
        tokio::time::timeout(REQUEST_TIMEOUT, request)
            .await
            .map_err(StoreError::from)
    }

    /// Whether `name` is a plain identifier, which statements quote safely.
    fn identifier(name: &str) -> bool {
        let mut chars = name.chars();
        chars
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }

    /// Inserts `rows`, one JSON object per line, into `table`. A refusal keeps
    /// only ClickHouse's error code, since its message can quote the rows.
    pub(crate) async fn insert(&self, table: &'static str, rows: Bytes) -> Result<(), StoreError> {
        let mut insert = self
            .client
            .insert_formatted_with(format!("INSERT INTO {table} FORMAT JSONEachRow"));
        let sent = Self::within(async move {
            insert.send(rows).await?;
            insert.end().await
        })
        .await?;
        sent.map_err(|error| match error {
            clickhouse::error::Error::Network(_) => StoreError::Unreachable(error),
            error => StoreError::Insert {
                table,
                code: Self::code(&error),
            },
        })
    }

    /// The error code ClickHouse's answer starts with, `Code: 60. DB::Exception:
    /// …`, without the message, or `unknown`.
    fn code(error: &clickhouse::error::Error) -> String {
        let clickhouse::error::Error::BadResponse(message) = error else {
            return "unknown".to_owned();
        };
        let digits: String = message
            .strip_prefix("Code: ")
            .unwrap_or_default()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if digits.is_empty() {
            "unknown".to_owned()
        } else {
            digits
        }
    }

    /// `query`, a fixed query of auth's own, to read with its `{name:Type}`
    /// placeholders bound by ClickHouse: [`Read::param`] binds each, and
    /// [`Read::rows`] reads them. Its text is `'static`, so no value is ever
    /// spliced into it, and `?fields` names the columns of the row type.
    #[must_use]
    pub fn query(&self, query: &'static str) -> Read {
        Read {
            query,
            request: self.client.query(query),
        }
    }

    /// Runs one statement, e.g. a `CREATE TABLE`, in `database`. `shown` is how
    /// errors quote it. It is sent as it is: a `?` in a comment binds nothing.
    async fn execute(
        &self,
        database: &str,
        statement: &str,
        shown: &str,
    ) -> Result<(), StoreError> {
        let client = self.client.clone().with_database(database);
        Self::within(client.query_raw(statement).execute())
            .await?
            .map_err(|source| StoreError::Statement {
                statement: shown.to_owned(),
                source,
            })
    }

    /// Creates the database, and every table of `tables` and its own `spans`
    /// and `logs` that does not exist, adds the columns a table lacks, and sets
    /// each one's retention and comments; then, when connected as the owner,
    /// creates or resets `writer`, allowed only to insert, and `reader`, allowed
    /// only to read the tables of `tables`, and every table's name and size,
    /// within limits on each query. Running it again changes nothing. A column's
    /// type and a table's sort order never change: give a new type a new column,
    /// and a new order a new table.
    ///
    /// # Errors
    /// Returns the first statement ClickHouse refuses.
    pub async fn migrate(
        &self,
        tables: &[Table],
        writer: Option<&Self>,
        reader: Option<&Self>,
    ) -> Result<(), StoreError> {
        let create = format!("CREATE DATABASE IF NOT EXISTS {}", self.database);
        self.execute("system", &create, &create).await?;
        for table in tables.iter().chain(crate::export::TABLES) {
            for statement in table.statements() {
                self.execute(&self.database, &statement, &statement).await?;
            }
        }
        let database = &self.database;
        if let Some(writer) = writer {
            self.provision(writer, &format!("INSERT ON {database}.*"), None)
                .await?;
        }
        if let Some(reader) = reader {
            let mut grants: Vec<String> = tables
                .iter()
                .map(|table| format!("SELECT ON {database}.{}", table.name))
                .collect();
            grants.push(format!("SHOW TABLES ON {database}.*"));
            grants.push(READER_SYSTEM.to_owned());
            self.provision(reader, &grants.join(", "), Some(READER_LIMITS))
                .await?;
        }
        Ok(())
    }

    /// Creates or resets `user`'s SQL user with its password, then gives it
    /// `grants`, such as `INSERT ON sourceorigin.*`, replacing any other right
    /// at once, so a running replica never lacks one, and `limits` on its
    /// queries.
    async fn provision(
        &self,
        user: &Self,
        grants: &str,
        limits: Option<&str>,
    ) -> Result<(), StoreError> {
        let (name, database) = (&user.user, &self.database);
        let password = Self::quoted(user.password.expose_secret());
        let identified = format!("IDENTIFIED WITH sha256_password BY {password}");
        let settings = limits.map_or_else(
            || "SETTINGS NONE".to_owned(),
            |limits| format!("SETTINGS {limits}"),
        );
        for (statement, shown) in [
            (
                format!("CREATE USER IF NOT EXISTS {name} {identified}"),
                format!("CREATE USER IF NOT EXISTS {name}"),
            ),
            (
                format!("ALTER USER {name} {identified} {settings}"),
                format!("ALTER USER {name} {settings}"),
            ),
            (
                format!("GRANT {grants} TO {name} WITH REPLACE OPTION"),
                String::new(),
            ),
        ] {
            let shown = if shown.is_empty() { &statement } else { &shown };
            self.execute(database, &statement, shown).await?;
        }
        Ok(())
    }

    /// `text` as a ClickHouse string literal.
    fn quoted(text: &str) -> String {
        let mut quoted = String::with_capacity(text.len() + 2);
        quoted.push('\'');
        for c in text.chars() {
            if matches!(c, '\'' | '\\') {
                quoted.push('\\');
            }
            quoted.push(c);
        }
        quoted.push('\'');
        quoted
    }
}

/// A fixed query and the values bound to it, which [`Store::query`] starts.
pub struct Read {
    /// The query, as errors quote it.
    query: &'static str,
    /// The request, with its values.
    request: clickhouse::query::Query,
}

impl Read {
    /// Binds `value` to the `{name:Type}` placeholder `name`, as its type
    /// writes it, such as an `i64` for an `Int64`.
    #[must_use]
    pub fn param(self, name: &str, value: impl serde::Serialize) -> Self {
        Self {
            request: self.request.param(name, value),
            ..self
        }
    }

    /// The rows, each a `T`, which the client checks against the names and
    /// types of the columns ClickHouse returns.
    ///
    /// # Errors
    /// Returns what ClickHouse refused the query with, or the column that does
    /// not match `T`.
    pub async fn rows<T: clickhouse::RowOwned + clickhouse::RowRead>(
        self,
    ) -> Result<Vec<T>, StoreError> {
        let query = self.query;
        Store::within(self.request.fetch_all())
            .await?
            .map_err(|source| StoreError::Query { query, source })
    }
}

impl Table {
    /// The statements that bring ClickHouse's table to this declaration.
    pub(crate) fn statements(&self) -> Vec<String> {
        let name = self.name;
        let comment = |lines: &[&str]| {
            Store::quoted(
                &lines
                    .iter()
                    .map(|line| line.trim())
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        };
        let ttl = format!("toDateTime(time) + INTERVAL {} DAY", self.retention_days);
        let (precision, partition, index, settings) = match self.kind {
            TableKind::Signal => (3, "toYYYYMM", "", ""),
            TableKind::Telemetry => (
                6,
                "toYYYYMMDD",
                ",\n    INDEX trace_index trace TYPE bloom_filter GRANULARITY 1",
                "\nSETTINGS ttl_only_drop_parts = 1",
            ),
        };
        let mut create = format!(
            "CREATE TABLE IF NOT EXISTS {name} (\n    time DateTime64({precision}, 'UTC') CODEC(DoubleDelta, ZSTD)"
        );
        for column in self.columns {
            write!(
                create,
                ",\n    {} {} COMMENT {}",
                column.name,
                column.ty,
                comment(column.description)
            )
            .ok();
        }
        let order: Vec<&str> = self.order_by.iter().copied().chain(["time"]).collect();
        write!(
            create,
            "{index}\n) ENGINE = MergeTree\nPARTITION BY {partition}(time)\nORDER BY ({})\nTTL {ttl}{settings}\nCOMMENT {}",
            order.join(", "),
            comment(self.description),
        )
        .ok();
        let mut statements = vec![create];
        for column in self.columns {
            let (column, ty, text) = (column.name, column.ty, comment(column.description));
            statements.push(format!(
                "ALTER TABLE {name} ADD COLUMN IF NOT EXISTS {column} {ty} COMMENT {text}"
            ));
            statements.push(format!("ALTER TABLE {name} COMMENT COLUMN {column} {text}"));
        }
        statements.push(format!(
            "ALTER TABLE {name} MODIFY COMMENT {}",
            comment(self.description)
        ));
        statements.push(format!(
            "ALTER TABLE {name} MODIFY TTL {ttl} SETTINGS materialize_ttl_after_modify = 0"
        ));
        statements
    }
}

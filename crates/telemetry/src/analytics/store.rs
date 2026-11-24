//! ClickHouse through its own client, `clickhouse`, over its HTTP interface:
//! inserts as `JSONEachRow`, and the statements `migrate` runs.

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
    /// Fails when a variable is malformed, a password cannot be read, or the
    /// owner is set without the writer's password.
    pub fn migration_from_environment() -> Result<Option<(Self, Option<Self>)>, ConfigurationError>
    {
        let Some(writer) = Self::from_environment()? else {
            return Ok(None);
        };
        Ok(Some(match Self::read(OWNER_URL, OWNER_PASSWORD)? {
            Some(owner) => (owner, Some(writer)),
            None => (writer, None),
        }))
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
    /// creates or resets `writer`, allowed only to insert. Running it again
    /// changes nothing. A column's type and a table's sort order never change:
    /// give a new type a new column, and a new order a new table.
    ///
    /// # Errors
    /// Returns the first statement ClickHouse refuses.
    pub async fn migrate(&self, tables: &[Table], writer: Option<&Self>) -> Result<(), StoreError> {
        let create = format!("CREATE DATABASE IF NOT EXISTS {}", self.database);
        self.execute("system", &create, &create).await?;
        for table in tables.iter().chain(crate::export::TABLES) {
            for statement in table.statements() {
                self.execute(&self.database, &statement, &statement).await?;
            }
        }
        if let Some(writer) = writer {
            let user = &writer.user;
            let password = Self::quoted(writer.password.expose_secret());
            for (statement, shown) in [
                (
                    format!(
                        "CREATE USER IF NOT EXISTS {user} IDENTIFIED WITH sha256_password BY {password}"
                    ),
                    format!("CREATE USER IF NOT EXISTS {user}"),
                ),
                (
                    format!("ALTER USER {user} IDENTIFIED WITH sha256_password BY {password}"),
                    format!("ALTER USER {user}"),
                ),
                (
                    format!(
                        "GRANT INSERT ON {}.* TO {user} WITH REPLACE OPTION",
                        self.database
                    ),
                    String::new(),
                ),
            ] {
                let shown = if shown.is_empty() { &statement } else { &shown };
                self.execute(&self.database, &statement, shown).await?;
            }
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

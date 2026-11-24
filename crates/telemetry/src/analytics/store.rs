//! ClickHouse over its HTTP interface, through the workspace's `reqwest`: inserts
//! as `JSONEachRow`, and the statements `migrate` runs.

use super::Table;
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
    /// Sends the requests.
    client: reqwest::Client,
    /// The server's HTTP interface, without user or path.
    endpoint: Url,
    /// The user, a plain identifier.
    user: String,
    /// The user's password.
    password: SecretString,
    /// The database, a plain identifier.
    database: String,
}

/// Why ClickHouse refused or never answered a request. It never holds what
/// ClickHouse answered an insert with, which can quote the rows.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The request did not complete, such as when ClickHouse is unreachable.
    #[error("could not reach ClickHouse")]
    Unreachable(#[from] reqwest::Error),
    /// ClickHouse refused an insert, with its error code.
    #[error("ClickHouse refused the insert into {table} (code {code})")]
    Insert {
        /// The table.
        table: &'static str,
        /// ClickHouse's error code, such as `60` for a missing table.
        code: String,
    },
    /// ClickHouse refused a statement of `migrate`, with its message.
    #[error("ClickHouse refused `{statement}`: {message}")]
    Statement {
        /// The statement, without any password it set.
        statement: String,
        /// What ClickHouse answered.
        message: String,
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
    /// needs the rights to create tables itself. `None` without `ANALYTICS_URL`.
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
    fn new(
        variable: Variable,
        value: &str,
        password: SecretString,
    ) -> Result<Self, ConfigurationError> {
        const SHAPE: &str = "must be an http:// or https:// URL naming a user and a database, \
                             such as http://USER@HOST:8123/DATABASE, without a password";
        let mut endpoint = Url::parse(value).map_err(|source| variable.error(SHAPE, source))?;
        let user = endpoint.username().to_owned();
        let database = endpoint.path().trim_matches('/').to_owned();
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || !Self::identifier(&user)
            || !Self::identifier(&database)
        {
            return Err(variable.invalid(SHAPE));
        }
        endpoint.set_path("/");
        endpoint.set_username("").ok();
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|source| variable.error("could not build the HTTP client", source))?;
        Ok(Self {
            client,
            endpoint,
            user,
            password,
            database,
        })
    }

    /// The database `TEST_ANALYTICS_URL` names, as its owner, with the
    /// `TEST_ANALYTICS_PASSWORD` password; `just analytics scratch` sets both.
    ///
    /// # Panics
    /// Panics when either is unset or malformed.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "a test helper, which panics as documented"
    )]
    pub fn test() -> Self {
        let url = Variable::new("TEST_ANALYTICS_URL");
        let value = url
            .required()
            .expect("TEST_ANALYTICS_URL names a scratch ClickHouse");
        let password = Variable::new("TEST_ANALYTICS_PASSWORD")
            .required()
            .expect("TEST_ANALYTICS_PASSWORD is its owner's password");
        Self::new(url, &value, SecretString::from(password)).expect("a valid TEST_ANALYTICS_URL")
    }

    /// The same server as another user, e.g. the writer `migrate` provisions.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn as_user(&self, user: &str, password: &str) -> Self {
        Self {
            user: user.to_owned(),
            password: SecretString::from(password),
            ..self.clone()
        }
    }

    /// The first column of `query`'s rows, as text, one row per line.
    ///
    /// # Errors
    /// Returns what ClickHouse refused the query with.
    #[cfg(any(test, feature = "test-util"))]
    pub async fn select(&self, query: &str) -> Result<String, StoreError> {
        let response = self
            .send(None, Bytes::from(format!("{query} FORMAT TSV")))
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if status.is_success() {
            Ok(text.trim_end().to_owned())
        } else {
            Err(StoreError::Statement {
                statement: query.to_owned(),
                message: text.trim().to_owned(),
            })
        }
    }

    /// Whether `name` is a plain identifier, which statements quote safely.
    fn identifier(name: &str) -> bool {
        let mut chars = name.chars();
        chars
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }

    /// Inserts `rows`, one JSON object per line, into `table`.
    pub(crate) async fn insert(&self, table: &'static str, rows: Bytes) -> Result<(), StoreError> {
        let query = format!("INSERT INTO {table} FORMAT JSONEachRow");
        let response = self.send(Some(&query), rows).await?;
        if response.status().is_success() {
            return Ok(());
        }
        let code = response
            .headers()
            .get("x-clickhouse-exception-code")
            .and_then(|code| code.to_str().ok())
            .unwrap_or("unknown")
            .to_owned();
        Err(StoreError::Insert { table, code })
    }

    /// Runs one statement, e.g. a `CREATE TABLE`. `shown` is how errors quote it.
    async fn execute(&self, statement: &str, shown: &str) -> Result<(), StoreError> {
        let response = self.send(None, Bytes::from(statement.to_owned())).await?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(StoreError::Statement {
            statement: shown.to_owned(),
            message: response.text().await?.trim().to_owned(),
        })
    }

    /// Sends `body` to this database: the statement itself, or the rows that
    /// `insert`, an `INSERT` statement, reads.
    async fn send(
        &self,
        insert: Option<&str>,
        body: Bytes,
    ) -> Result<reqwest::Response, StoreError> {
        let mut url = self.endpoint.clone();
        url.query_pairs_mut()
            .extend_pairs(insert.map(|query| ("query", query)))
            .append_pair("database", &self.database);
        Ok(self
            .client
            .post(url)
            .header("x-clickhouse-user", &self.user)
            .header("x-clickhouse-key", self.password.expose_secret())
            .body(body)
            .send()
            .await?)
    }

    /// Creates every table of `tables` that does not exist, adds the columns
    /// a table lacks, and sets each one's retention and comments; then, when
    /// connected as the owner, creates or resets `writer`, allowed only to insert.
    /// Running it again changes nothing. A column's type never changes: give
    /// the new type a new column.
    ///
    /// # Errors
    /// Returns the first statement ClickHouse refuses.
    pub async fn migrate(&self, tables: &[Table], writer: Option<&Self>) -> Result<(), StoreError> {
        for table in tables {
            for statement in table.statements() {
                self.execute(&statement, &statement).await?;
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
                (format!("REVOKE ALL ON *.* FROM {user}"), String::new()),
                (
                    format!("GRANT INSERT ON {}.* TO {user}", self.database),
                    String::new(),
                ),
            ] {
                let shown = if shown.is_empty() { &statement } else { &shown };
                self.execute(&statement, shown).await?;
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
    pub(super) fn statements(&self) -> Vec<String> {
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
        let mut create = format!(
            "CREATE TABLE IF NOT EXISTS {name} (\n    time DateTime64(3, 'UTC') CODEC(DoubleDelta, ZSTD)"
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
            "\n) ENGINE = MergeTree\nPARTITION BY toYYYYMM(time)\nORDER BY ({})\nTTL {ttl}\nCOMMENT {}",
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

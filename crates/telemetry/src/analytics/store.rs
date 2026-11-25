//! ClickHouse over its HTTP interface, through the workspace's `reqwest`: inserts
//! as `JSONEachRow`, and the statements `migrate` runs.

use super::Table;
use bytes::Bytes;
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
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
    /// ClickHouse refused a query, with its message.
    #[error("ClickHouse refused `{query}`: {message}")]
    Query {
        /// The query.
        query: &'static str,
        /// What ClickHouse answered.
        message: String,
    },
    /// A query's rows did not decode as asked.
    #[error("the rows of `{query}` do not decode")]
    Decode {
        /// The query.
        query: &'static str,
        /// Why a row did not decode.
        source: serde_json::Error,
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
        let response = self
            .send(
                &self.database,
                &[],
                Bytes::from(format!("{query} FORMAT TSV")),
            )
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

    /// The rows `query` returns, each decoded as a `T` from a JSON object, with
    /// `parameters` bound to its `{name:Type}` placeholders by ClickHouse. The
    /// query's text is `'static`, so no value is ever spliced into it.
    ///
    /// # Errors
    /// Returns what ClickHouse refused the query with, or a row that does not
    /// decode.
    pub async fn query<T: DeserializeOwned>(
        &self,
        query: &'static str,
        parameters: &[(&str, &str)],
    ) -> Result<Vec<T>, StoreError> {
        let parameters: Vec<(String, &str)> = parameters
            .iter()
            .map(|(name, value)| (format!("param_{name}"), *value))
            .collect();
        let parameters: Vec<(&str, &str)> = parameters
            .iter()
            .map(|(name, value)| (name.as_str(), *value))
            .collect();
        let body = Bytes::from(format!("{query}\nFORMAT JSONEachRow"));
        let response = self.send(&self.database, &parameters, body).await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(StoreError::Query {
                query,
                message: text.trim().to_owned(),
            });
        }
        text.lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()
            .map_err(|source| StoreError::Decode { query, source })
    }

    /// Inserts `rows`, one JSON object per line, into `table`.
    pub(crate) async fn insert(&self, table: &'static str, rows: Bytes) -> Result<(), StoreError> {
        let query = format!("INSERT INTO {table} FORMAT JSONEachRow");
        let response = self
            .send(&self.database, &[("query", &query)], rows)
            .await?;
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

    /// Runs one statement, e.g. a `CREATE TABLE`, in `database`. `shown` is how
    /// errors quote it.
    async fn execute(
        &self,
        database: &str,
        statement: &str,
        shown: &str,
    ) -> Result<(), StoreError> {
        let response = self
            .send(database, &[], Bytes::from(statement.to_owned()))
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(StoreError::Statement {
            statement: shown.to_owned(),
            message: response.text().await?.trim().to_owned(),
        })
    }

    /// Sends `body` to `database`, with `parameters` in the URL: the statement
    /// itself, or the rows that a `query` parameter, an `INSERT` statement,
    /// reads.
    async fn send(
        &self,
        database: &str,
        parameters: &[(&str, &str)],
        body: Bytes,
    ) -> Result<reqwest::Response, StoreError> {
        let mut url = self.endpoint.clone();
        url.query_pairs_mut()
            .extend_pairs(parameters)
            .append_pair("database", database);
        Ok(self
            .client
            .post(url)
            .header("x-clickhouse-user", &self.user)
            .header("x-clickhouse-key", self.password.expose_secret())
            .body(body)
            .send()
            .await?)
    }

    /// Creates the database, its `spans` and `logs` tables with their
    /// retention, and every table of `tables` that does not exist, adds the
    /// columns a signal's table lacks, and sets each one's retention and
    /// comments; then, when connected as the owner, creates or resets `writer`,
    /// allowed only to insert, and `reader`, allowed only to read the tables of
    /// `tables`, and every table's name and size, within limits on each query. Running it
    /// again changes nothing. A column's type and a table's sort order never
    /// change: give a new type a new column, and a new order a new table.
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
        for statement in crate::export::TABLES {
            self.execute(&self.database, statement, statement).await?;
        }
        for table in tables {
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

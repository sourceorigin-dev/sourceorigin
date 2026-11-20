# sourceorigin-repository

Typed PostgreSQL persistence for the authentication service. SQLx pools and SQL
remain private; callers use repositories or a `DatabaseTransaction`.

The schema lives in numbered, immutable files under `migrations/`. Each repository
module in `src/repositories/` owns its methods and record types; `mod.rs` holds
only shared plumbing. See [database architecture](../../docs/dev/architecture/database.md)
for the layout and invariants. Run migrations through the single service binary:
`just db migrate`.

Every query is a checked SQLx macro, verified at compile time against the
committed cache in `.sqlx/`: after changing a query or a migration, run
`just db prepare` and commit the cache with the change.

Tests that need PostgreSQL are ignored by `just test`; `just db test` runs them
in a scratch cluster, then checks the cache against the migrated schema.
[Testing](../../docs/dev/testing.md#database-tests) describes them.

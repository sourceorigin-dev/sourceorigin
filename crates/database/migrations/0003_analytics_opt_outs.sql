-- The accounts whose owners left usage statistics: auth records none of their
-- activity as product analytics, such as which kind of passkey they sign in
-- with (docs/dev/architecture/analytics.md#consent). No row means the default,
-- included. Deleting an account deletes its row.
CREATE TABLE analytics_opt_outs (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    opted_out_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

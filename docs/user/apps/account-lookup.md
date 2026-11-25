# Naming accounts from their IDs

Keep an account's ID in your records, and ask Origin Auth for the name to show
beside it. The ID never changes, so your records stay right when someone
renames their account, and stop naming them once the account is gone.

## Before you start

- You need the account's ID: the `sub` of the tokens your app receives when a
  user signs in. It is the same for every app, and it never changes.
- The lookup is public. It needs no client, no secret and no token, and any
  site's page may call it.

## Look accounts up

Ask for up to 100 accounts in one request, one `id` parameter each:

```console
curl "https://auth.sourceorigin.dev/accounts?id=0199a000-0000-7000-8000-000000000001&id=0199a000-0000-7000-8000-000000000002"
```

Origin Auth answers with the accounts it can name, in no particular order:

```json
{
  "accounts": [
    { "id": "0199a000-0000-7000-8000-000000000001", "handle": "ada" }
  ]
}
```

- `handle` is the account's handle now. After a rename, the same ID answers
  with the new one.
- An account that is disabled, scheduled for deletion or deleted is left out,
  and so is an ID no account has. The answer doesn't say which: show such an ID
  as a former account, without a name.

With the Rust SDK, which asks for 100 accounts at a time:

```rust
let accounts = issuer.accounts(&[note.author]).await?;
match accounts.first() {
    Some(account) => println!("by @{}", account.handle),
    None => println!("by a former account"),
}
```

With the TypeScript SDK:

```ts
const [author] = await issuer.accounts([note.authorId]);
const byline = author ? `@${author.handle}` : 'a former account';
```

## Keep names fresh

Look names up when you show them, rather than storing them with your records.
You may keep an answer for a short while to save requests, but an account that
is disabled or deleted should stop being named, so keep it for minutes, not
days.

## Limits

- A lookup names 1 to 100 accounts. More, none, or an `id` that isn't an
  account ID, answers `400` with what is wrong.
- Lookups are limited for each IP address, to 120 a minute on
  auth.sourceorigin.dev, apart from every other kind of request. Past it, the
  answer is `429`.
- The lookup returns the handle only. An account's display name, picture and
  email address reach an app only when its user signs in to it.

# Calling an API as your app

Get an access token for your backend itself, with no user involved, to call an
API from a worker, a scheduled job or another service. Your backend presents
its client secret, and Origin Auth answers with a token that names the app.
This is the OAuth client credentials grant (RFC 6749 §4.4).

## Before you start

- You need an [app with a backend](register-an-app.md), which has a client
  secret. An app without one can't use this grant: anyone who knew its client
  ID could ask for its tokens.
- Turn on **Client credentials** under **Grant types** in the app's
  [settings](app-settings.md). An operator's client gets it with
  `--grant-type client_credentials`: see
  [OIDC clients](../../dev/operations/clients.md).
- The app needs access to the [API it calls](apis.md), with the scopes it uses.
  The API's owner gives it, on the API's page.

The SDKs don't run this grant yet
([#60](https://github.com/sourceorigin-dev/sourceorigin/issues/60)). The request
below is plain HTTPS, and `curl` shows it.

## Ask for the token

Post to the `token_endpoint` of the [discovery document](../reference/oidc.md),
with your client ID and secret as HTTP Basic credentials:

```console
curl https://auth.sourceorigin.dev/oauth/token \
    -u "$SOURCEORIGIN_CLIENT_ID:$SOURCEORIGIN_CLIENT_SECRET" \
    -d grant_type=client_credentials \
    -d resource=https://notes.example/api \
    -d scope="notes:read"
```

Origin Auth answers with the token alone:

```json
{
  "access_token": "eyJhbGciOiJSUzI1NiIs…",
  "token_type": "Bearer",
  "expires_in": 300,
  "scope": "notes:read"
}
```

- `resource` is the API's identifier. You can leave it out when your app has
  access to exactly one API.
- `scope` lists the scopes you need, among those the API's owner gave your
  app. Leave it out to get all of them.
- There is no refresh token and no ID token: nobody signed in, and your secret
  gets the next token.

Send the token to the API as `Authorization: Bearer <token>`.

## Keep the token until it expires

The token works for `expires_in` seconds, five minutes. Keep it in memory and
reuse it for every call, then ask for another shortly before it expires. Don't
ask for one per call: Origin Auth limits token requests for each IP address, to
120 a minute on auth.sourceorigin.dev, and every service behind one address
shares that limit. Past it, the answer is `429`.

## Tell an app from a person in your API

The token's `sub` is your app's client ID, where a user's token carries the
user's account ID. It has no `sid`, and says nothing of a sign-in. Both SDKs
read this for you when they [validate the token](protect-an-api.md):

- In Rust, `token.subject()` is `Subject::Client(client_id)`, and
  `Subject::User(user_id)` for a person.
- In TypeScript, `token.subject` is `{ kind: 'client', id }`, and
  `{ kind: 'user', id }` for a person.

Decide what each may do: a scope says what a token allows, and the subject says
who is asking.

## Check that it works

Decode the access token. Its `sub` and `client_id` are both your client ID, its
`aud` is the API's identifier, and its `scope` lists what you asked for.

## Troubleshooting

| `error` | Why | What to do |
| --- | --- | --- |
| `invalid_client` | The client ID or secret is wrong, the secret was replaced by a rotation, or the app is disabled. | Check both, and [rotate the secret](rotate-secrets.md) if it is lost. |
| `unauthorized_client` | The app isn't registered for this grant. | Turn on **Client credentials** in the app's settings. |
| `invalid_target` | `resource` names an API the app has no access to, or it was left out and the app has access to none, or to several. | Name the API, and ask its owner for access. |
| `invalid_scope` | The app wasn't granted one of the scopes for this API. `openid`, `profile`, `email` and `offline_access` are never granted here: they describe a user. | Ask for the API's own scopes, or ask its owner for more. |

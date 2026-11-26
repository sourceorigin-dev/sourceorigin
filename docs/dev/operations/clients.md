# OIDC clients

This page helps you register the apps that sign in with Origin Auth, change them,
rotate their secrets, and stop one that misbehaves, and register the APIs their
tokens are for.

Clients live in the database. A change applies to the next request on every
replica, without a restart or a deployment, and is recorded as an event
attributed to you. Manage them:

- **At the terminal**, with `auth-service client` (`just auth client` locally;
  in an environment, see
  [running auth-service commands](deploying.md#running-auth-service-commands)).
- **On the admin dashboard**, on the **Clients** page.

Accounts register clients for their own apps in the developer console, at
`/developers` on auth: see [registering an app](../../user/apps/register-an-app.md)
and [clients accounts register](#clients-accounts-register).

## Register a client

At the terminal, a first-party app running in the browser:

```sh
auth-service client create blog --name Blog --trusted --reason "SourceOrigin's blog" \
    --redirect-uri https://blog.sourceorigin.dev/callback --post-logout-uri https://blog.sourceorigin.dev/
```

An app with a server, which keeps a secret and may refresh tokens:

```sh
auth-service client create notes --name Notes --confidential --reason "Notes app" \
    --redirect-uri https://notes.example/callback --scope openid --scope offline_access
```

A command-line tool, which signs users in with a code they approve on another
device, and stays signed in:

```sh
auth-service client create notes-cli --name "Notes CLI" --reason "Notes' command-line tool" \
    --scope openid --scope offline_access \
    --grant-type urn:ietf:params:oauth:grant-type:device_code --grant-type refresh_token
```

A worker, which calls an API as itself:

```sh
auth-service client create notes-worker --name "Notes worker" --confidential --reason "Notes' worker" \
    --grant-type client_credentials
auth-service resource grant notes-worker https://notes.sourceorigin.dev/api --scope notes:read --reason "Notes' worker"
```

A client with the authorization code grant lists at least one redirect URI.
The command-line tool and the worker have other grants only, so they list none.

For a confidential client, the command prints its secret once: store it in the
app's configuration straight away.

On the dashboard:

1. Open **Clients**, and select **Register a client**.
2. Fill in its **Client ID**, which never changes, its settings, and a
   **Reason**.
3. Select **Register**. The next page shows a confidential client's secret,
   once.

[Settings](#settings) explains each option.

## Change a client

```sh
auth-service client show notes     # its registration and its secrets' lifetimes
auth-service client update blog --add-redirect-uri https://blog.sourceorigin.dev/auth/callback --reason "New callback"
auth-service client update notes --backchannel-logout-uri https://notes.example/auth/backchannel-logout --reason "Logout"
```

`update` changes only what you give it. On the dashboard, open the client's
settings, change them, give a **Reason**, and select **Save changes**. Every
change to a client needs a reason, recorded with its event.

## Rotate a client's secret

1. Give the client a new secret:

   ```sh
   auth-service client rotate-secret notes --grace 24h --reason "Yearly rotation"
   ```

   On the dashboard, open the client and select **Rotate the secret**, choose
   how long **Earlier secrets keep working**, give a **Reason**, and select
   **Rotate secret**.
2. Store the new secret, printed once, in the app's configuration, and deploy
   the app.
3. Check with `auth-service client show notes` that the earlier secrets end
   when you expect.

The app's earlier secrets keep working for the grace period, 24 hours by
default and 30 days at most, so you can switch the app over without downtime.
`--grace 0`, or **Stop them now**, revokes them at once, and asks first: for a secret that
leaked, see [a leaked client secret](incidents.md#a-leaked-client-secret).

## Disable, suspend or delete a client

- **Disable** stops the client signing users in, exchanging codes and
  refreshing tokens, at once. `client enable` undoes it.
- **Suspend** disables a client so that its owner can't enable it again until
  `client unsuspend`, which leaves it disabled. Use it for an account's client
  that breaks the rules, such as one impersonating another app. Its owner sees
  that it was suspended.
- **Delete** revokes the client's refresh tokens and removes its secrets and
  consents. Its ID stays reserved forever, since tokens carry it as `aud`, and
  its name and URIs are erased.

> **Warning:** deleting a client can't be undone.

```sh
auth-service client disable notes --reason "Leaking tokens"
auth-service client suspend notes --reason "Phishing"
auth-service client delete notes --reason "Retired"
```

Each asks for confirmation. On the dashboard, open the client and select
**Disable**, **Suspend** or **Delete**.

## Apply a client a deployment manages

For a client that a deployment registers itself, run `client apply`, with the
same options as `create`, from a deployment job:

```sh
auth-service client apply blog --name Blog --confidential \
    --redirect-uri https://blog.sourceorigin.dev/callback --secret file:///run/secrets/blog-client-secret \
    --reason "Deployment"
```

It registers the client, or updates what differs, and records nothing when
nothing does, so the job can run on every deployment. Its secret comes from the
deployment, through `--secret file://<path>` or `env://<name>`: 32 random bytes
in unpadded base64url, never printed. The client's other secrets keep working
for `--grace` at most, 1 hour by default, while the deployment rolls out;
`--grace 0` revokes them. Only the terminal applies clients, since only it takes
a secret. Like `create`, it gives a client the defaults of its kind for the
scopes and grant types it doesn't name, so a confidential client applied
without `--scope` or `--grant-type` gets `offline_access` and refresh tokens. To
keep them off, name its scopes and pass `--grant-type authorization_code`.

No deployment of SourceOrigin needs a client: the admin dashboard signs
administrators in itself (see [the admin dashboard](administration.md#how-the-dashboard-signs-administrators-in)).

## Approve an origin for passkey sign-in

A trusted client's own pages may sign users in with their passkeys, without
sending them to auth's pages, from origins you approve:

```sh
auth-service client origin add blog https://blog.sourceorigin.dev --reason "Blog sign-in"
auth-service client origin list
auth-service client origin remove blog https://blog.sourceorigin.dev --reason "Blog moved"
```

The client must be trusted, with a redirect URI on the origin. Approved origins
may span 5 domains at most, across every client, as browsers honour no more. On the
dashboard, open the client and select **Add approved origin** under **Passkey
sign-in**. See [passkey sign-in on your own pages](../../user/apps/passkey-sign-in.md).

## Settings

- **Kind.** Clients are public by default, and authenticate with PKCE alone
  (`none`). Confidential clients (`--confidential`) use `client_secret_basic`
  with a secret auth generates. Public clients must send S256 PKCE; confidential
  clients may leave it out, and a challenge they send is enforced.
- **Redirect URIs** must match exactly. They use HTTPS, or HTTP on a loopback IP
  for native apps, without credentials or a fragment. Write them as URL parsing
  normalizes them: `https://app.example/`, not `https://APP.example`. The
  origins of an enabled public client's redirect URIs may call the token,
  revocation and userinfo endpoints from a browser.
- **Scopes** are those of Origin Auth's own the client may request: `openid`,
  `profile`, `email` and `offline_access`. They default to the first three, and
  `offline_access` for a confidential client, which the SDKs' login routes
  always request. An API's scopes aren't a client setting: the client gets them
  with its [access to the API](#apis).
- **Grant types** (`--grant-type`, once per grant) are the grants the client
  may use at the token endpoint, which RFC 7591 calls its `grant_types`:
  `authorization_code`, `refresh_token`,
  `urn:ietf:params:oauth:grant-type:device_code` and `client_credentials`. A client starts an
  authorization and redeems its code only with `authorization_code`. With
  `refresh_token`, it receives refresh tokens when it also requests
  `offline_access`. With the device grant, it signs devices without a browser
  in ([signing in on a device](../../user/apps/device-sign-in.md)). With
  `client_credentials`, a confidential client gets tokens for itself, with no
  user, for the [APIs](#apis) it has access to
  ([calling an API as your app](../../user/apps/service-tokens.md)); a public
  client is refused it. A public
  client defaults to `authorization_code` alone, and a confidential one to
  both. Naming any replaces the list, so name every grant the client keeps.
  The dashboard's and the developer console's forms start with the same
  defaults for the kind chosen.
- **Trusted** (`--trusted`) skips the consent screen. It is for first-party
  clients only.
- **Pushed requests** (`--require-par`) make the client push its authorization
  requests, so none of their parameters travel through the browser.
  `update --no-require-par` lifts it.
- **Back-channel logout** (`--backchannel-logout-uri`) is where auth posts a
  logout token when a session the client holds tokens from ends: see
  [back-channel logout](../../user/apps/back-channel-logout.md). It uses HTTPS,
  and an IP address host must be public. A name is checked when auth delivers,
  since what it resolves to can change. A deleted client is sent none.

## APIs

An API that access tokens are issued for is a resource: an identifier, which
its tokens carry as `aud`, the scopes it defines, and the clients it admits.
A client names it in the `resource` parameter of its request (RFC 8707).
Accounts register their own in the developer console: see
[registering an API](../../user/apps/apis.md). You register SourceOrigin's own,
and see and change every one.

Register an API, define a scope, and let a client request it:

```sh
auth-service resource create https://notes.sourceorigin.dev/api --name "Notes API" --reason "Notes' API"
auth-service resource scope add https://notes.sourceorigin.dev/api notes:read \
    --description "Read your notes" --reason "Notes' API"
auth-service resource grant notes https://notes.sourceorigin.dev/api --scope notes:read --reason "Notes' own app"
```

The identifier is an `https` URI without query or fragment, written as the API
compares it. It can't change later. `grant` replaces the scopes the client could
request before, so name every scope it keeps.

On the dashboard:

1. Open **Resources**, and select **Register a resource**.
2. Fill in its **Identifier**, its **Name** and a **Reason**, and select
   **Register**.
3. Open the resource. Select **Define a scope** for each scope, then **Grant a
   client access** for each client.

See what is registered, and change it:

```sh
auth-service resource list
auth-service resource show https://notes.sourceorigin.dev/api    # its scopes, and the clients it admits
auth-service resource rename https://notes.sourceorigin.dev/api --name Notes --reason "Renamed"
auth-service resource scope remove https://notes.sourceorigin.dev/api notes:read --reason "Unused"
auth-service resource revoke notes https://notes.sourceorigin.dev/api --reason "Retired"
```

On the dashboard, the resource's page has **Rename**, and **Remove** and
**Revoke** beside each scope and client.

- **Removing a scope** takes it from what every client may request. A refresh
  token that holds it is refused at its next use.
- **Revoking a client's access** revokes its refresh tokens for the API.

> **Warning:** deleting a resource can't be undone. No client gets tokens for
> it any more, every refresh token for it is revoked, and users' consents to it
> are removed.

```sh
auth-service resource delete https://notes.sourceorigin.dev/api --reason "Retired"
```

It asks for confirmation. On the dashboard, open the resource and select
**Delete**.

An account can have 10 resources, each with 32 scopes at most, and can't name
one after a reserved brand. Purging an account deletes its resources.

## Secrets

`create` and `rotate-secret` print a secret once, and auth stores only its
SHA-256 digest. Secrets start with `socs_`, so secret scanners can recognize
them.

## Clients accounts register

Auth generates the IDs of the clients accounts register in the developer
console. They are never trusted. Their names can't contain SourceOrigin's or
another well-known brand (`RESERVED_BRANDS` in `sourceorigin_common::clients`), or a
look-alike: `sourceorigin_common::similarity` reads digits such as `0` as letters,
and letters of other scripts, such as a Cyrillic `о`, as the Latin ones they look
like. An account can have 10, and register 20 a day.

They never appear in other accounts' consoles, while `client list` and the
dashboard show every client. Owners can disable, enable and delete their
clients, and rotate their secrets. Deleting a client, from either side, disables
it, revokes its refresh tokens and removes its secrets and consents.

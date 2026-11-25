# Handles are public

A handle names an account in public. It is the account's login name, relying
parties receive it as `preferred_username`, and published profiles will live at
`sourceorigin.dev/@handle` ([#101](https://github.com/sourceorigin-dev/sourceorigin/issues/101)).
Auth doesn't hide which handles have an account:

- Handle sign-in refuses a handle without an active account at once: "No active
  account has this handle." (`start_login` in `apps/auth-service/src/passkey/login.rs`).
- Signup and renames say why a handle can't be taken, naming the look-alike
  that blocks it (`Handles::claimable` in `apps/auth-service/src/handles/mod.rs`).

This was decided in [#255](https://github.com/sourceorigin-dev/sourceorigin/issues/255).

Auth also names an account from its ID. `GET /accounts` answers the current
handle of each active account among the IDs it is given
(`apps/auth-service/src/directory`), to anyone, so that services can keep IDs
in their records and show names
([#773](https://github.com/sourceorigin-dev/sourceorigin/issues/773)).
[From an ID to a handle](#from-an-id-to-a-handle) says what that gives away.

## Why handles aren't hidden

- **Uniqueness gives handles away.** Signup has to refuse a handle that's taken,
  and so do renames. Anyone who can sign up, or who holds an account, can
  therefore check any handle. A vaguer refusal would only stop naming the
  look-alike; it would still say the typed handle is unavailable. Production
  requires invitations, which limits signup to people with one, but open signup
  would extend this to everyone.
- **Handles are already published:** to relying parties, and in profiles once
  #101 ships.
- **Decoys would hide little.** Answering handle sign-in for an unknown handle
  with stable fake credentials, derived from a server key, gives them fixed
  shapes, so accounts with other passkeys stand out. A disabled account would
  visibly switch to decoys, and every change of the key changes them, so anyone
  comparing responses from before and after learns which handles exist
  ([#181](https://github.com/sourceorigin-dev/sourceorigin/issues/181)).

## What a handle reveals

| Who | Learns |
| --- | --- |
| Anyone, through handle sign-in | Whether the handle belongs to an account that can sign in with a passkey by handle: one that is active, or scheduled for deletion, and has an unrevoked passkey. Nothing about its passkeys ([below](#credential-ids-stay-private)). |
| Anyone admitted to signup, and every account holder through a rename | Whether the handle, or a look-alike, is held or was held, and which look-alike. |
| Relying parties the user signs in to | The handle, as `preferred_username`. |
| Anyone who holds an account's ID, through the lookup | The account's current handle, while the account is active, and so every rename. Whether an ID that doesn't resolve was never an account, or is one that is disabled or deleted, stays hidden. |
| Anyone, once a profile is published (#101) | The profile. #101 has an unpublished profile answer 404 like an unknown handle, so it reveals nothing beyond what sign-in does. |

The account picker (signing in without a handle) reveals nothing: the
authenticator names the account.

## Credential IDs stay private

No sign-in lists an account's credential IDs. Handle sign-in starts the same
discoverable ceremony as the account picker, with an empty `allowCredentials`.
Only a ceremony for someone who already holds the account lists them: a passkey
confirmation in a signed-in session, and the undo of a recovery from the owner's
link.
The authenticator names the account in its answer, and auth refuses a passkey
of any account other than the one the handle named (`verify` in
`apps/auth-service/src/passkey/login.rs`). It says so only once the passkey
signed: the account an assertion names is the client's claim, so a forged one
fails as any bad signature does, and sign-in confirms no guess of the handle an
account's ID has now. The lookup answers that question outright, for an active
account ([from an ID to a handle](#from-an-id-to-a-handle)). A related origin's sign-in for the
account its client hints at works the same way.

Listing the IDs would undo a rename. They are stable and unique, and a rename
doesn't change them. Anyone who recorded an account's IDs before a rename could
try candidate handles afterwards and find the same IDs, linking the old handle to
the new one. That link is often why people rename, and
[#101](https://github.com/sourceorigin-dev/sourceorigin/issues/101) refuses to
redirect renamed handles for that reason. The IDs would also reveal how many passkeys the
account has and, from their lengths, often which provider holds them
([WebAuthn's privacy note](https://www.w3.org/TR/webauthn-3/#sctn-credential-id-privacy-leak)).

So every passkey must be discoverable: an authenticator that can't name its
account would have no sign-in left. Registration requires a resident key, and
an authenticator that can't store one, such as an old security key or one whose
slots are full, refuses to create the passkey. Discoverable credentials on
security keys and on other devices still sign in: the browser's prompt offers
them.

This was decided in [#297](https://github.com/sourceorigin-dev/sourceorigin/issues/297).
The alternative kept listing the IDs and warned in the settings that a rename
doesn't hide the old handle unless the owner also replaces their passkeys.

## From an ID to a handle

Before the lookup, a handle led to nothing else, and nothing led to a handle:
an account's ID reached only the relying parties its owner signed in to, as
`sub`, and they learned a new handle only at the owner's next sign-in. The
lookup adds the other direction. Whoever holds an account's ID can ask for its
current handle at any time, without the owner signing in anywhere.

**A rename no longer hides from someone who holds the ID.** The section above
keeps credential IDs private so that nobody can link an old handle to a new
one. That still holds for someone who knows only the old handle: no endpoint
takes a handle and answers an ID. It doesn't hold for someone who holds the
account's ID, who now follows every rename. Who holds an ID:

- every relying party the owner signed in to, which also receives the handle
  at each sign-in;
- anyone a service shows the ID to. A service that prints account IDs in its
  pages or its API hands that link to all its readers. Services that only show
  the name the lookup answers don't.

So an owner who renames to cut the link to an old handle is still linked for
the services that recorded their ID, and for whoever those services show it
to. A new account is the only way to cut that link.

What the lookup doesn't give away:

- **No enumeration.** IDs are UUIDv7: the time of signup, then random bits,
  far too many to guess. Nobody finds accounts by guessing IDs, and the budget
  of lookups for each IP address bounds the tries.
- **Nothing but the handle.** No display name, picture, email address, or date.
  An ID's own timestamp says when the account was created, to whoever holds it.
- **No status.** A disabled account, one scheduled for deletion, a purged one
  and an ID that never existed all answer the same: they are left out. A holder
  of an ID that used to resolve does learn that the account stopped being
  active, as a relying party learns at a refused refresh.

Only active accounts resolve, so disabling or deleting an account stops every
service naming it at its next lookup. The answer is not cacheable.

## Threat model

Knowing that a handle has an account gives an attacker little:

- **Nothing to guess.** Sign-in needs a passkey. A handle doesn't help forge
  one, and passkeys resist phishing.
- **No lockout.** Rate limits count requests per client IP, never per account
  (`apps/auth-service/src/security/http.rs`), so naming an account can't lock its
  owner out.
- **Not evidence.** Support's evidence policy never counts knowing a handle
  ([account recovery](account-recovery.md#supports-evidence-policy)).
- **No credential IDs.** Sign-in lists no account's credentials, so a handle
  says nothing about its passkeys.

The cost is privacy: anyone can check whether a person, by the username they use
elsewhere, has an Origin Account. For the sign-in of SourceOrigin's own apps, that
is accepted.

## Limits

- **Anyone can add failed sign-ins to an account's history.** Handle sign-in
  binds its ceremony to the account, so a failed assertion, even from a passkey
  that was never registered, is recorded against it as `SignInFailed`. The
  owner's history hides these, but support reads them in the account's events.
  Per-IP rate limits slow a single source, but nothing bounds the failures across
  sources. The failures change nothing: they are noise, not progress.

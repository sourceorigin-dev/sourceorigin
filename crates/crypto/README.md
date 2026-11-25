# sourceorigin-crypto

The cryptography [Origin Auth](https://auth.sourceorigin.dev) and its Rust SDK
share, so both judge keys and signatures the same way: secrets and digests, PKCE,
RS256 signing and verification of JSON Web Tokens, AES-256-GCM sealing, signed
notes, and the SCRAM-SHA-256 verifiers of auth's database roles. It builds on [aws-lc-rs](https://docs.rs/aws-lc-rs), which needs a C
compiler.

To sign users in or validate tokens, use
[`sourceorigin-auth`](https://crates.io/crates/sourceorigin-auth) instead, which
depends on this crate.

## Features

With no features, the crate is what the Rust SDK uses: random secrets, digests,
HMAC, RS256 verification and AES-256-GCM sealing. `server` adds what only auth
uses, and compiles nothing unless enabled: RS256 signing and key sets, Ed25519
signed notes, SCRAM-SHA-256 verifiers, the key-encryption key, client secrets
and PKCE.

It needs Rust 1.88 or newer. The
[API documentation](https://docs.rs/sourceorigin-crypto) describes each primitive.

## Versions

`sourceorigin-common`, `sourceorigin-crypto` and `sourceorigin-auth` are released
together, at the same version, and depend on each other at exactly that version.
[The releases](https://github.com/sourceorigin-dev/sourceorigin/releases) list the
changes in each.

## License

[Apache-2.0](LICENSE-APACHE).

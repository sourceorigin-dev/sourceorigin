# sourceorigin-common

The types [Origin Auth](https://auth.sourceorigin.dev) and its SDKs share:
identifiers, string vocabularies, and the wire types of auth's protocol, such as
token claims, discovery metadata and Bearer challenges.

To sign users in or validate tokens, use
[`sourceorigin-auth`](https://crates.io/crates/sourceorigin-auth) instead, which
depends on this crate and re-exports what an app needs from it.

## Features

With no features, the crate is what the SDKs build on; the Rust SDK adds
`client-spans`, for a span around each request it sends. The others are for
SourceOrigin's own servers, and compile nothing unless enabled: `server`,
`admin`, `config`, `email`, `events`, `similarity` and `sqlx`. `server` holds
what auth and sourceorigin.dev share beyond the SDKs: client registrations
(`clients`), handles (`handle`), the public IP ranges (`net`), auth's settings
API bodies and response headers, the IDs and vocabularies of accounts and their
records, and the `str_enum!` and `open_str_enum!` macros. `admin`, `events` and
`similarity` enable it. The
[API documentation](https://docs.rs/sourceorigin-common) describes each.

It needs Rust 1.88 or newer.

## Versions

`sourceorigin-common`, `sourceorigin-crypto` and `sourceorigin-auth` are released
together, at the same version, and depend on each other at exactly that version.
[The releases](https://github.com/sourceorigin-dev/sourceorigin/releases) list the
changes in each.

## License

[Apache-2.0](LICENSE-APACHE).

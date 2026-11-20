//! The limits, lifetimes and names every SDK applies the same way, so their tokens,
//! key caches and login cookies behave alike. The TypeScript SDK reads them from its
//! generated declarations, durations in milliseconds.

use chrono::TimeDelta;
use std::time::Duration;

super::sdk_constants! {
    /// Clock skew tolerated on `exp`, `nbf`, `iat` and `auth_time`. Auth tolerates the
    /// same on `iat` alone, and none on `exp` or `max_age`.
    pub const LEEWAY: TimeDelta = TimeDelta::seconds(30);
    /// Longest a logout token may be valid: auth signs each delivery attempt for two
    /// minutes, so a longer-lived token was not made for one attempt.
    pub const MAX_LOGOUT_LIFETIME: TimeDelta = TimeDelta::minutes(10);
    /// Longest compact token read, in bytes: decoding is bounded before it starts.
    pub const MAX_TOKEN_BYTES: usize = 16 * 1024;
    /// Shortest RSA modulus a key may have, in bits (RFC 7518 §3.3).
    pub const MIN_MODULUS_BITS: usize = 2048;
    /// Longest RSA modulus a key may have, in bits, as aws-lc verifies.
    pub const MAX_MODULUS_BITS: usize = 8192;
    /// `typ` media types may carry this prefix (RFC 7515 §4.1.9).
    pub const MEDIA_TYPE_PREFIX: &str = "application/";

    /// Shortest time between two key fetches, whatever triggers them. Bounds how often
    /// unknown key IDs or an outage can make the cache call the issuer.
    pub const MIN_KEY_FETCH_INTERVAL: TimeDelta = TimeDelta::seconds(30);
    /// How long keys keep verifying once they stop being fresh, while every refresh
    /// fails. Stale keys may still trust a revoked key, so they get the same bound
    /// auth gives its own keys.
    pub const KEYS_STALE_AFTER: TimeDelta = TimeDelta::minutes(15);
    /// How long fetched keys stay fresh when the response sets no `max-age`.
    pub const DEFAULT_KEYS_LIFETIME: TimeDelta = TimeDelta::hours(1);
    /// Longest time fetched keys stay fresh, whatever `max-age` says.
    pub const MAX_KEYS_LIFETIME: TimeDelta = TimeDelta::hours(24);

    /// Most bytes read from any response. Metadata, key sets and token responses are a
    /// few kilobytes.
    pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
    /// Longest a request may take, from connecting or sending it to its last body
    /// byte, whatever the HTTP client's own timeout.
    pub const REQUEST_DEADLINE: Duration = Duration::from_secs(10);

    /// Access tokens this close to expiry are refreshed first.
    pub const REFRESH_MARGIN: TimeDelta = TimeDelta::seconds(30);
    /// Longest a refresh may take, auth's response and its ID token check included.
    pub const REFRESH_DEADLINE: Duration = Duration::from_secs(20);
    /// Longest a session's revocation may hold up the sign-in or logout ending it,
    /// whatever the HTTP client's own timeout.
    pub const REVOCATION_DEADLINE: Duration = Duration::from_secs(5);
    /// How long after a step-up sign-in the browser has to follow the redirect back.
    /// A refusal within it is that step-up's one retry; a later one is a new action.
    pub const STEP_UP_RETRY: TimeDelta = TimeDelta::minutes(1);
    /// How long a login may take at auth.
    pub const PENDING_LOGIN_LIFETIME: TimeDelta = TimeDelta::minutes(10);
    /// Default session lifetime.
    pub const DEFAULT_SESSION_LIFETIME: TimeDelta = TimeDelta::hours(12);
    /// Most bytes in a `return_to` path.
    pub const MAX_RETURN_TO_BYTES: usize = 2048;
    /// Longest client ID and scope, in bytes.
    pub const MAX_IDENTIFIER_BYTES: usize = 64;

    /// Starts the name of every login cookie, which goes on with the client ID, so
    /// apps sharing a host keep their own. `__Host-` makes browsers require `Secure`,
    /// `Path=/` and no `Domain`, so no other host can set or read them.
    pub const COOKIE_PREFIX: &str = "__Host-sourceorigin-";
    /// Most bytes in a cookie's name, `=` and value: browsers drop larger cookies.
    pub const MAX_COOKIE_BYTES: usize = 4096;
    /// Cookie slots for logins in progress: each login waits in the slot its `state`
    /// hashes to, replacing any login there. Responses to logins started at once cannot
    /// see each other's cookies, so only fixed slots bound how many a browser keeps.
    pub const PENDING_LOGIN_SLOTS: u8 = 3;
    /// Most bytes in one pending login cookie's name and value, so all slots together
    /// take at most 3000 bytes. Common proxies refuse a `Cookie` header over 8 KiB, which
    /// they share with the session, up to 4 KiB, and the app's own cookies. A login
    /// whose `return_to` would outgrow it returns to the home page instead.
    pub const MAX_PENDING_COOKIE_BYTES: usize = 1000;
    /// Bytes in a cookie key, and in the AES-256 keys derived from it.
    pub const COOKIE_KEY_BYTES: usize = 32;
    /// The HKDF `info` of the key sealing sessions. Each names the format version: a new
    /// format gets a new `info`, so cookies of the old one no longer open, and every SDK
    /// uses the same, so their cookies interoperate.
    pub const SESSION_SEAL_INFO: &str = "sourceorigin-auth session v1";
    /// The HKDF `info` of the key sealing pending logins, versioned as
    /// [`SESSION_SEAL_INFO`] is.
    pub const PENDING_LOGIN_SEAL_INFO: &str = "sourceorigin-auth pending login v1";
}

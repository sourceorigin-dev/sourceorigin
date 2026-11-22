//! The leeway policy, the order of the checks, and the audience rules. Every check
//! runs at a fixed instant long past, so a check reading the system clock would fail.
use super::*;
use serde_json::{Value, json};

/// The instant every check here runs at.
const NOW: i64 = 1_767_225_600;
/// The issuer every token here names.
const ISSUER: &str = "https://auth.example";

/// ID token claims issued a minute before [`NOW`] and valid for five more, with the
/// `changes` applied.
fn id_token(changes: &Value) -> IdTokenClaims {
    let mut claims = json!({"iss": ISSUER, "sub": "user", "aud": "client", "iat": NOW - 60,
        "exp": NOW + 300, "nonce": "nonce"});
    for (claim, value) in changes.as_object().expect("changes") {
        claims[claim] = value.clone();
    }
    serde_json::from_value(claims).expect("ID token claims")
}

/// What an SDK checks an ID token against.
fn validation() -> Validation<'static> {
    Validation {
        issuers: &[ISSUER],
        audience: Some("client"),
        nonce: Some("nonce"),
        expiry: Expiry::Lenient,
        now: DateTime::from_timestamp(NOW, 0).expect("time"),
    }
}

/// `exp` allows the leeway only where `Expiry` says, and `nbf` and `iat` always do,
/// each up to its boundary.
#[test]
fn leeway_follows_the_policy() {
    let seconds = LEEWAY.num_seconds();
    let at = |offset: i64| DateTime::from_timestamp(NOW + offset, 0).expect("time");
    for (expiry, exp, result) in [
        (Expiry::Lenient, NOW - seconds + 1, Ok(())),
        (
            Expiry::Lenient,
            NOW - seconds,
            Err(ClaimsError::Expired(at(-seconds))),
        ),
        (Expiry::Strict, NOW + 1, Ok(())),
        (Expiry::Strict, NOW, Err(ClaimsError::Expired(at(0)))),
        (Expiry::Ignored, NOW - 3600, Ok(())),
    ] {
        let check = Validation {
            expiry,
            ..validation()
        };
        assert_eq!(
            check.check(&id_token(&json!({"exp": exp}))),
            result,
            "{expiry:?}"
        );
    }
    for expiry in [Expiry::Lenient, Expiry::Strict, Expiry::Ignored] {
        let check = Validation {
            expiry,
            ..validation()
        };
        for (claim, error) in [
            ("nbf", ClaimsError::NotYetValid as fn(_) -> _),
            ("iat", ClaimsError::IssuedInFuture),
        ] {
            assert_eq!(
                check.check(&id_token(&json!({claim: NOW + seconds}))),
                Ok(())
            );
            assert_eq!(
                check.check(&id_token(&json!({claim: NOW + seconds + 1}))),
                Err(error(at(seconds + 1))),
                "{claim}"
            );
        }
    }
}

/// A token with several faults gets the first one's error, and a far-future `exp`
/// never overflows into an expiry.
#[test]
fn checks_run_in_order() {
    let faults = [
        ("iss", json!("https://other.example")),
        ("aud", json!("other")),
        ("exp", json!(NOW - 3600)),
        ("nbf", json!(NOW + 3600)),
        ("iat", json!(NOW + 3600)),
        ("nonce", json!("other")),
    ];
    let reasons = [
        ErrorReason::IssuerMismatch,
        ErrorReason::WrongAudience,
        ErrorReason::Expired,
        ErrorReason::NotYetValid,
        ErrorReason::IssuedInFuture,
        ErrorReason::NonceMismatch,
    ];
    for (first, reason) in reasons.into_iter().enumerate() {
        let changes = Value::Object(
            faults[first..]
                .iter()
                .map(|(claim, value)| ((*claim).to_owned(), value.clone()))
                .collect(),
        );
        let result = validation().check(&id_token(&changes));
        assert_eq!(result.map_err(|error| error.reason()), Err(reason));
    }
    let far = id_token(&json!({"exp": DateTime::<Utc>::MAX_UTC.timestamp()}));
    assert_eq!(validation().check(&far), Ok(()));
}

/// Any accepted issuer passes; an ID token must name its client alone, an access
/// token among others, and no audience is checked when none is expected. A nonce is
/// required only when expected.
#[test]
fn issuers_audiences_and_nonces() {
    let google = Validation {
        issuers: &["https://accounts.google.com", "accounts.google.com"],
        ..validation()
    };
    let claims = id_token(&json!({"iss": "accounts.google.com"}));
    assert_eq!(google.check(&claims), Ok(()));
    assert!(matches!(
        validation().check(&claims),
        Err(ClaimsError::WrongIssuer { found, .. }) if found == "accounts.google.com"
    ));

    let shared = id_token(&json!({"aud": ["client", "other"]}));
    assert!(matches!(
        validation().check(&shared),
        Err(ClaimsError::WrongAudience { .. })
    ));
    let access: AccessTokenClaims = serde_json::from_value(json!({"iss": ISSUER,
        "sub": "user", "aud": ["client", "other"], "client_id": "client",
        "jti": "01936f3e-8c00-7000-8000-000000000003", "scope": "openid",
        "iat": NOW - 60, "exp": NOW + 300}))
    .expect("access token claims");
    let unbound = Validation {
        nonce: None,
        ..validation()
    };
    assert_eq!(unbound.check(&access), Ok(()));
    let anyone = Validation {
        audience: None,
        ..unbound
    };
    assert_eq!(anyone.check(&id_token(&json!({"aud": "other"}))), Ok(()));

    let no_nonce = id_token(&json!({"nonce": null}));
    assert_eq!(anyone.check(&no_nonce), Ok(()));
    assert_eq!(
        validation().check(&no_nonce),
        Err(ClaimsError::NonceMismatch)
    );
}

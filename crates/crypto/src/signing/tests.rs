//! JWT policy, tampering, protected-header trust boundaries and key selection.
use super::*;
use serde_json::{Value, json};
use sourceorigin_common::protocol::{claims::AccessTokenClaims, registered::ClaimsError};

/// The issuer every token here names.
const ISSUER: &str = "https://auth.example";

/// The instant every check here runs at: long past, so a check reading the system
/// clock would refuse every token.
fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_600, 0).expect("time")
}

/// ID token claims, issued at [`now`] and valid for five minutes, with `changes`.
/// They also hold the claims an access token needs, so they sign one too.
fn claims(changes: &Value) -> Value {
    let now = now().timestamp();
    let mut claims = json!({"iss":ISSUER,"sub":"user","aud":"client","iat":now,"exp":now+300,
        "client_id":"client","jti":"01936f3e-8c00-7000-8000-000000000003","scope":"openid"});
    for (claim, value) in changes.as_object().expect("changes") {
        claims[claim] = value.clone();
    }
    claims
}

/// Verification checks the type, signature and issuer, and the times at the time
/// it is given: `exp` without leeway, except for hints, which may have expired,
/// and `nbf` and `iat` with it.
#[test]
fn verifies_types_signatures_issuers_and_times_at_the_given_time() {
    let keys = KeySet::single(Rs256Key::generate("test").expect("generate RSA test key"));
    let at = |seconds: i64| now() + chrono::TimeDelta::seconds(seconds);
    let verify = |claims: &Value| {
        let token = keys.sign(TokenType::Id, claims).expect("sign");
        (
            keys.verify::<IdTokenClaims>(&token, TokenType::Id, ISSUER, now()),
            keys.verify_id_token_hint(&token, ISSUER, now()),
        )
    };
    let (token, hint) = verify(&claims(&json!({})));
    assert_eq!(token.expect("verify").sub, "user");
    assert_eq!(hint.expect("hint").sub, "user");

    // Expired a second ago: refused as a token, read as a hint.
    let (token, hint) = verify(&claims(&json!({"exp": at(-1).timestamp()})));
    assert!(matches!(
        token,
        Err(CryptoError::Claims(ClaimsError::Expired(exp))) if exp == at(-1)
    ));
    assert!(hint.is_ok());
    for changes in [
        json!({"iss": "https://evil.example"}),
        json!({"nbf": at(31).timestamp()}),
        json!({"iat": at(31).timestamp()}),
    ] {
        let (token, hint) = verify(&claims(&changes));
        assert!(matches!(token, Err(CryptoError::Claims(_))), "{changes}");
        assert!(matches!(hint, Err(CryptoError::Claims(_))), "{changes}");
    }
    for changes in [
        json!({"nbf": at(30).timestamp()}),
        json!({"iat": at(30).timestamp()}),
    ] {
        let (token, hint) = verify(&claims(&changes));
        assert!(token.is_ok() && hint.is_ok(), "{changes} within the leeway");
    }
    let mut no_exp = claims(&json!({}));
    no_exp.as_object_mut().expect("claims").remove("exp");
    let (token, hint) = verify(&no_exp);
    assert!(matches!(token, Err(CryptoError::InvalidClaims(_))));
    assert!(matches!(hint, Err(CryptoError::InvalidClaims(_))));

    let token = keys
        .sign(TokenType::Access, &claims(&json!({})))
        .expect("sign access token");
    assert!(
        keys.verify::<AccessTokenClaims>(&token, TokenType::Access, ISSUER, now())
            .is_ok()
    );
    assert!(matches!(
        keys.verify::<IdTokenClaims>(&token, TokenType::Id, ISSUER, now()),
        Err(CryptoError::InvalidToken)
    ));
    let mut bytes = token.into_bytes();
    let n = bytes.len();
    bytes[n - 10] = if bytes[n - 10] == b'A' { b'B' } else { b'A' };
    let tampered = std::str::from_utf8(&bytes).expect("modified compact token remains ASCII");
    assert!(matches!(
        keys.verify::<AccessTokenClaims>(tampered, TokenType::Access, ISSUER, now()),
        Err(CryptoError::Signature)
    ));
}

/// Even correctly RSA-signed inputs cannot choose an algorithm, type, key or critical extension.
#[test]
fn rejects_algorithm_confusion_and_unrecognized_headers() {
    let keys = KeySet::single(Rs256Key::generate("headers").expect("RSA key"));
    let key = keys.signer();
    let claims = claims(&json!({}));
    for (field, value) in [
        ("alg", json!("none")),
        ("alg", json!("HS256")),
        ("alg", json!("RS512")),
        ("typ", json!("at+jwt")),
        ("kid", json!("untrusted")),
        ("crit", json!([])),
        ("crit", json!(["b64"])),
    ] {
        let mut header =
            json!({"alg":Algorithm::Rs256.as_str(), "kid":key.kid, "typ":TokenType::Id.as_str()});
        header[field] = value;
        let input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).expect("header")),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("claims"))
        );
        // A valid RSA signature isolates header-policy rejection from signature failure.
        let token = key.signed(&input).expect("signature");
        assert!(
            matches!(
                keys.verify::<IdTokenClaims>(&token, TokenType::Id, ISSUER, now()),
                Err(CryptoError::InvalidToken | CryptoError::UnknownKey | CryptoError::Json(_))
            ),
            "accepted {field}"
        );
    }
    // HMAC with public-key bytes is the classic asymmetric/symmetric algorithm confusion.
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","kid":"headers","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("claims"));
    let input = format!("{header}.{payload}");
    let signature = Digest::hmac(key.public_key().pem().as_bytes(), input.as_bytes()).encoded();
    let token = format!("{input}.{signature}");
    assert!(matches!(
        keys.verify::<IdTokenClaims>(&token, TokenType::Id, ISSUER, now()),
        Err(CryptoError::Json(_))
    ));
}

/// During a rotation, tokens of every published key verify and JWKS lists them;
/// a retired key verifies only ID token hints, and a key outside the set nothing.
#[test]
fn key_set_verifies_by_kid_and_limits_retired_keys_to_hints() {
    let [retiring, active, retired, unknown] = ["retiring", "active", "retired", "unknown"]
        .map(|kid| Rs256Key::generate(kid).expect("key"));
    let claims = claims(&json!({}));
    let [by_retiring, by_retired, by_unknown] =
        [&retiring, &retired, &unknown].map(|key| key.sign(TokenType::Id, &claims).expect("sign"));
    // A signer must be published, or verifiers could not check its tokens.
    assert!(matches!(
        KeySet::new(unknown, vec![retiring.public_key().clone()], vec![]),
        Err(CryptoError::KeySet)
    ));
    let published = vec![retiring.public_key().clone(), active.public_key().clone()];
    let keys = KeySet::new(active, published, vec![retired.public_key().clone()]).expect("key set");
    let by_active = keys.sign(TokenType::Id, &claims).expect("sign");
    for token in [&by_active, &by_retiring] {
        assert!(
            keys.verify::<IdTokenClaims>(token, TokenType::Id, ISSUER, now())
                .is_ok()
        );
        assert!(keys.verify_id_token_hint(token, ISSUER, now()).is_ok());
    }
    assert!(matches!(
        keys.verify::<IdTokenClaims>(&by_retired, TokenType::Id, ISSUER, now()),
        Err(CryptoError::UnknownKey)
    ));
    assert!(
        keys.verify_id_token_hint(&by_retired, ISSUER, now())
            .is_ok()
    );
    assert!(matches!(
        keys.verify_id_token_hint(&by_unknown, ISSUER, now()),
        Err(CryptoError::UnknownKey)
    ));
    let kids: Vec<_> = keys
        .jwks()
        .keys
        .iter()
        .map(|key| key.kid.as_str())
        .collect();
    assert_eq!(kids, ["retiring", "active"]);

    // A repeated key identifier, even between published and retired keys, is refused.
    let signer = Rs256Key::generate("same").expect("key");
    let twin = Rs256Key::generate("same").expect("key");
    let published = vec![signer.public_key().clone()];
    assert!(matches!(
        KeySet::new(signer, published, vec![twin.public_key().clone()]),
        Err(CryptoError::KeySet)
    ));
}

/// A private key exports as the PKCS#8 PEM `openssl` writes, byte for byte, so
/// a key read from a fixture writes back the same file. The export fills the
/// buffer it was allocated, so it never reallocated and left a copy unzeroized.
#[test]
fn private_key_pem_writes_back_the_pem_it_was_read_from() {
    let pem = include_str!("../../../../tests/sdk-vectors/keys/key-2025.pem");
    let key = Rs256Key::from_pem("2025", pem).expect("fixture key");
    let exported = key.private_key_pem().expect("export PEM");
    assert_eq!(exported.as_str(), pem);
    assert_eq!(exported.capacity(), exported.len());
}

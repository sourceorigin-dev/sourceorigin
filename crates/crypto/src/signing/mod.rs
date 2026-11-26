//! RSA signing for interoperable `OpenID` Connect ID and access tokens, and the key
//! set that signs with one key and verifies with every key it publishes.
use super::{CryptoError, Digest};
use crate::{Compact, Rs256VerifyingKey, VerifyError};
use aws_lc_rs::{
    encoding::{AsDer, Pkcs8V1Der},
    rand::SystemRandom,
    rsa::KeySize,
    signature::{KeyPair, RSA_PKCS1_SHA256, RsaKeyPair},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::{DateTime, Utc};
use secrecy::zeroize::Zeroizing;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sourceorigin_common::protocol::{
    claims::IdTokenClaims,
    jwt::{Algorithm, Jwk, JwkSet, KeyId, KeyType, KeyUse, TokenType},
    registered::{Expiry, RegisteredClaims, Validation},
};
use std::{borrow::Cow, collections::BTreeSet};

/// RSA key size used by generated keys.
const GENERATED_RSA_SIZE: KeySize = KeySize::Rsa3072;
/// The PEM label of a PKCS#8 private key.
const PKCS8_LABEL: &str = "PRIVATE KEY";
/// The PEM label of a public key.
const PUBLIC_KEY_LABEL: &str = "PUBLIC KEY";

/// Protected JOSE header. It is the only header auth writes and the only one it
/// accepts back: unknown fields, `crit` included, fail parsing.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header<'a> {
    /// Signing algorithm; a value outside [`Algorithm`] fails parsing.
    alg: Algorithm,
    /// Key that signed the token.
    kid: Cow<'a, str>,
    /// Token type, which keeps ID and access tokens apart.
    typ: TokenType,
}

/// A compact token with auth's header parsed, but nothing verified.
struct Token<'a> {
    /// The parsed protected header.
    header: Header<'static>,
    /// The token's parts.
    compact: Compact<'a>,
}

impl<'a> Token<'a> {
    /// Splits a bounded token into exactly three parts and parses its header.
    fn parse(token: &'a str) -> Result<Self, CryptoError> {
        let split = Compact::split(token).map_err(CryptoError::MalformedToken)?;
        Ok(Self {
            header: serde_json::from_slice(split.header())?,
            compact: split.decode().map_err(CryptoError::MalformedToken)?,
        })
    }
}

/// A public signing key, as JWKS publishes it: what verifies the tokens it signed.
#[derive(Clone)]
pub struct PublicKey {
    /// The published form.
    jwk: Jwk,
    /// The key that verifies its RS256 signatures, which has no defect.
    key: Rs256VerifyingKey,
}

impl PublicKey {
    /// Builds the key a JWK describes.
    ///
    /// # Errors
    /// Rejects malformed keys, keys every SDK would refuse to verify with, and
    /// invalid key identifiers.
    pub fn from_jwk(jwk: Jwk) -> Result<Self, CryptoError> {
        jwk.kid.parse::<KeyId>().map_err(CryptoError::KeyId)?;
        let key = match jwk.kty {
            KeyType::Rsa => Rs256VerifyingKey::new(
                &URL_SAFE_NO_PAD.decode(&jwk.n)?,
                &URL_SAFE_NO_PAD.decode(&jwk.e)?,
            ),
        };
        if let Some(defect) = key.defect() {
            return Err(CryptoError::UnusableKey(defect));
        }
        Ok(Self { jwk, key })
    }

    /// The key identifier tokens name in their `kid` header.
    #[must_use]
    pub fn kid(&self) -> &str {
        &self.jwk.kid
    }

    /// The key as JWKS publishes it.
    #[must_use]
    pub const fn jwk(&self) -> &Jwk {
        &self.jwk
    }

    /// The key's JWK thumbprint (RFC 7638): the SHA-256 digest of its required
    /// members, in lexicographic order and canonical form, so it names the key
    /// itself, whatever its `kid` or how its JWK spells it.
    #[must_use]
    pub fn thumbprint(&self) -> Digest {
        let members = match self.jwk.kty {
            KeyType::Rsa => format!(
                r#"{{"e":"{}","kty":"RSA","n":"{}"}}"#,
                URL_SAFE_NO_PAD.encode(self.key.exponent()),
                URL_SAFE_NO_PAD.encode(self.key.modulus()),
            ),
        };
        Digest::sha256(members.as_bytes())
    }

    /// Exports the key as PEM, `SubjectPublicKeyInfo` in the form `openssl`
    /// writes, for independent verifiers.
    #[must_use]
    pub fn pem(&self) -> String {
        Pem::encode(PUBLIC_KEY_LABEL, self.key.der().unwrap_or_default())
            .as_str()
            .to_owned()
    }

    /// Whether this key signs with `alg`. Requiring the header's algorithm to be
    /// one of the key's own rules out algorithm confusion (RFC 8725 §3.1).
    const fn signs_with(&self, alg: Algorithm) -> bool {
        match (self.jwk.kty, alg) {
            (KeyType::Rsa, Algorithm::Rs256) => true,
        }
    }

    /// Verifies the type, `T`'s, and signature of a token naming this key, decodes
    /// its claims as `T`, and checks them at `now` as auth checks its own tokens:
    /// from `issuer`, of any audience and nonce, expiring as `expiry` says.
    fn verified<T: RegisteredClaims + DeserializeOwned>(
        &self,
        token: &Token<'_>,
        issuer: &str,
        expiry: Expiry,
        now: DateTime<Utc>,
    ) -> Result<T, CryptoError> {
        if token.header.typ != T::TYPE || !self.signs_with(token.header.alg) {
            return Err(CryptoError::UnexpectedHeader);
        }
        self.key
            .verify(token.compact.signing_input(), token.compact.signature())
            .map_err(|error| match error {
                VerifyError::Unusable(defect) => CryptoError::UnusableKey(defect),
                VerifyError::BadSignature => CryptoError::Signature,
            })?;
        let claims: T =
            serde_json::from_slice(token.compact.payload()).map_err(CryptoError::InvalidClaims)?;
        Validation {
            issuers: &[issuer],
            audience: None,
            nonce: None,
            expiry,
            now,
        }
        .check(&claims)?;
        Ok(claims)
    }
}

/// An RSA private key that signs RS256 tokens under its key identifier.
pub struct Rs256Key {
    /// Key identifier written in every token's header.
    kid: String,
    /// Validated RSA private key.
    key: RsaKeyPair,
    /// The public half, derived once from the private key.
    public: PublicKey,
}

impl Rs256Key {
    /// Loads a PKCS#8 PEM RSA private key (`PRIVATE KEY`) under `kid`, for test
    /// fixtures.
    ///
    /// # Errors
    /// Returns an error for invalid PEM, a non-RSA or undersized key, or an invalid identifier.
    pub fn from_pem(kid: impl Into<String>, pem: &str) -> Result<Self, CryptoError> {
        let der = Pem::decode(pem, PKCS8_LABEL).ok_or(CryptoError::Pem)?;
        Self::from_key(kid.into(), RsaKeyPair::from_pkcs8(&der)?)
    }

    /// Loads a PKCS#8 DER RSA private key under `kid`, as [`Self::private_key_der`] exports it.
    ///
    /// # Errors
    /// Returns an error for invalid DER, a non-RSA or undersized key, or an invalid identifier.
    pub fn from_der(kid: impl Into<String>, der: &[u8]) -> Result<Self, CryptoError> {
        Self::from_key(kid.into(), RsaKeyPair::from_pkcs8(der)?)
    }

    /// Generates a 3072-bit RSA key using the configured cryptographic provider.
    ///
    /// # Errors
    /// Preserves provider failures and rejects invalid key identifiers.
    pub fn generate(kid: impl Into<String>) -> Result<Self, CryptoError> {
        Self::from_key(kid.into(), RsaKeyPair::generate(GENERATED_RSA_SIZE)?)
    }

    /// Validates RSA size and key identifier for every construction path. The
    /// provider refuses keys under 2048 bits before this runs.
    fn from_key(kid: String, key: RsaKeyPair) -> Result<Self, CryptoError> {
        let public = key.public_key();
        let public = PublicKey::from_jwk(Jwk {
            alg: Some(Algorithm::Rs256),
            e: URL_SAFE_NO_PAD.encode(public.exponent().big_endian_without_leading_zero()),
            key_ops: None,
            kid: kid.clone(),
            kty: KeyType::Rsa,
            n: URL_SAFE_NO_PAD.encode(public.modulus().big_endian_without_leading_zero()),
            key_use: Some(KeyUse::Signature),
        })?;
        Ok(Self { kid, key, public })
    }

    /// The key identifier tokens name in their `kid` header.
    #[must_use]
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// The public half, which verifies this key's tokens.
    #[must_use]
    pub const fn public_key(&self) -> &PublicKey {
        &self.public
    }

    /// Exports the private key as PKCS#8 DER, for sealing in key storage.
    ///
    /// # Errors
    /// Preserves encoding failures from the provider.
    pub fn private_key_der(&self) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        let der: Pkcs8V1Der<'_> = self.key.as_der()?;
        Ok(Zeroizing::new(der.as_ref().to_vec()))
    }

    /// Exports the private key as PKCS#8 PEM, for test fixtures.
    ///
    /// # Errors
    /// Preserves encoding failures from the provider.
    pub fn private_key_pem(&self) -> Result<Zeroizing<String>, CryptoError> {
        Ok(Pem::encode(PKCS8_LABEL, &self.private_key_der()?))
    }

    /// Signs claims as a compact RS256 token whose `typ` header is `token_type`.
    ///
    /// # Errors
    /// Preserves provider and serialization failures.
    pub fn sign(
        &self,
        token_type: TokenType,
        claims: &impl Serialize,
    ) -> Result<String, CryptoError> {
        let header = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Header {
            alg: Algorithm::Rs256,
            kid: Cow::Borrowed(&self.kid),
            typ: token_type,
        })?);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims)?);
        self.signed(&format!("{header}.{payload}"))
    }

    /// The compact token of `input`, `header.payload`, with its RS256 signature.
    fn signed(&self, input: &str) -> Result<String, CryptoError> {
        let mut signature = vec![0; self.key.public_modulus_len()];
        self.key.sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            input.as_bytes(),
            &mut signature,
        )?;
        Ok(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature)))
    }

    /// This key alone as a JSON Web Key Set, for fixtures that publish one key.
    #[must_use]
    pub fn jwks(&self) -> JwkSet {
        JwkSet {
            keys: vec![self.public.jwk.clone()],
        }
    }
}

/// The keys auth signs and verifies with: one signing key, the published keys,
/// whose tokens verify and which JWKS lists, and retired keys, which verify only
/// `id_token_hint`s, since a hint may outlive its key's publication. Each key
/// identifier appears once, and a token is checked only against the key it names.
pub struct SigningKeySet {
    /// The key that signs new tokens; one of `published`.
    signer: Rs256Key,
    /// Keys whose tokens verify, in JWKS order.
    published: Vec<PublicKey>,
    /// Keys that verify only `id_token_hint`s.
    retired: Vec<PublicKey>,
    /// The published keys as a JSON Web Key Set.
    jwks: JwkSet,
}

impl SigningKeySet {
    /// Builds a set that signs with `signer`, which `published` must contain.
    ///
    /// # Errors
    /// Returns [`CryptoError::KeySet`] if `signer` is not published as itself, or a
    /// key identifier appears twice.
    pub fn new(
        signer: Rs256Key,
        published: Vec<PublicKey>,
        retired: Vec<PublicKey>,
    ) -> Result<Self, CryptoError> {
        let mut kids = BTreeSet::new();
        if !published.iter().any(|key| key.jwk == signer.public.jwk)
            || !published
                .iter()
                .chain(&retired)
                .all(|key| kids.insert(key.kid()))
        {
            return Err(CryptoError::KeySet);
        }
        let jwks = JwkSet {
            keys: published.iter().map(|key| key.jwk.clone()).collect(),
        };
        Ok(Self {
            signer,
            published,
            retired,
            jwks,
        })
    }

    /// A set of one key, which signs and alone verifies.
    #[must_use]
    pub fn single(signer: Rs256Key) -> Self {
        Self {
            published: vec![signer.public.clone()],
            retired: Vec::new(),
            jwks: signer.jwks(),
            signer,
        }
    }

    /// The key that signs new tokens.
    #[must_use]
    pub const fn signer(&self) -> &Rs256Key {
        &self.signer
    }

    /// Signs claims with the signing key; see [`Rs256Key::sign`].
    ///
    /// # Errors
    /// Preserves provider and serialization failures.
    pub fn sign(
        &self,
        token_type: TokenType,
        claims: &impl Serialize,
    ) -> Result<String, CryptoError> {
        self.signer.sign(token_type, claims)
    }

    /// The published keys as a JSON Web Key Set.
    #[must_use]
    pub const fn jwks(&self) -> &JwkSet {
        &self.jwks
    }

    /// Verifies a compact token of `T`'s type signed by a published key, and decodes
    /// its claims as `T`, which pass the shared check at `now`: `issuer`, no leeway
    /// on `exp` ([`Expiry::Strict`]), and any audience.
    ///
    /// # Errors
    /// Returns [`CryptoError::UnexpectedHeader`] for a token of another type,
    /// [`CryptoError::UnknownKey`] for a key outside the published ones,
    /// [`CryptoError::InvalidClaims`] for claims that do not decode as `T`, and
    /// [`CryptoError::Claims`] for claims that fail the check; otherwise preserves
    /// provider and decoding failures and rejects invalid signatures.
    pub fn verify<T: RegisteredClaims + DeserializeOwned>(
        &self,
        token: &str,
        issuer: &str,
        now: DateTime<Utc>,
    ) -> Result<T, CryptoError> {
        let token = Token::parse(token)?;
        Self::find(&self.published, &token)?.verified(&token, issuer, Expiry::Strict, now)
    }

    /// Verifies an ID token hint signed by a published or retired key, and checks
    /// its claims at `now` as [`Self::verify`] does, except that it may have
    /// expired ([`Expiry::Ignored`]).
    ///
    /// # Errors
    /// As [`Self::verify`], with retired keys among the known ones.
    pub fn verify_id_token_hint(
        &self,
        token: &str,
        issuer: &str,
        now: DateTime<Utc>,
    ) -> Result<IdTokenClaims, CryptoError> {
        let token = Token::parse(token)?;
        Self::find(&self.published, &token)
            .or_else(|_| Self::find(&self.retired, &token))?
            .verified(&token, issuer, Expiry::Ignored, now)
    }

    /// The key among `keys` that `token` names.
    fn find<'k>(keys: &'k [PublicKey], token: &Token<'_>) -> Result<&'k PublicKey, CryptoError> {
        keys.iter()
            .find(|key| key.kid() == token.header.kid)
            .ok_or(CryptoError::UnknownKey)
    }
}

/// PEM (RFC 7468): base64 DER between `-----BEGIN <label>-----` and
/// `-----END <label>-----` lines.
struct Pem;

impl Pem {
    /// The DER of the first block labelled `label` in `text`, if it decodes.
    /// A private key's base64 and DER are zeroized when dropped; the base64 is
    /// gathered into a buffer allocated at the body's size, so it never
    /// reallocates and leaves no copy behind.
    fn decode(text: &str, label: &str) -> Option<Zeroizing<Vec<u8>>> {
        let begin = format!("-----BEGIN {label}-----");
        let end = format!("-----END {label}-----");
        let body = text.split_once(&begin)?.1.split_once(&end)?.0;
        let mut base64 = Zeroizing::new(String::with_capacity(body.len()));
        for piece in body.split_whitespace() {
            base64.push_str(piece);
        }
        STANDARD.decode(base64.as_str()).ok().map(Zeroizing::new)
    }

    /// `der` as a block labelled `label`, in lines of 64 characters. For a private
    /// key's sake, every buffer holding its base64 is zeroized when dropped, and
    /// the PEM is allocated at its final size, so it never reallocates and leaves
    /// a copy behind in freed memory.
    fn encode(label: &str, der: &[u8]) -> Zeroizing<String> {
        let base64 = Zeroizing::new(STANDARD.encode(der));
        let begin = format!("-----BEGIN {label}-----\n");
        let end = format!("-----END {label}-----\n");
        let mut pem = Zeroizing::new(String::with_capacity(
            begin.len() + base64.len() + base64.len().div_ceil(64) + end.len(),
        ));
        pem.push_str(&begin);
        let mut rest = base64.as_str();
        while !rest.is_empty() {
            let (line, tail) = rest.split_at(rest.len().min(64));
            pem.push_str(line);
            pem.push('\n');
            rest = tail;
        }
        pem.push_str(&end);
        pem
    }
}

#[cfg(test)]
mod tests;

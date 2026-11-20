//! Writes `packages/auth/src/protocol.ts`, the TypeScript SDK's declarations of these
//! wire types, with ts-rs. Without `UPDATE_SDK_TYPES=1` it only compares, so
//! `just test` fails while the committed file is stale; `just sdk-types` rewrites it.
//!
//! The structs derive `TS` under `cfg(test)`, so ts-rs stays a dev-dependency. Types
//! that serialize by hand implement it here, from the same values serde writes:
//! vocabularies as unions of their spellings, validated strings as `string`. Each
//! closed vocabulary also gets the list of its spellings, which decoders check
//! values against.

use super::{
    challenge::{BearerChallenge, BearerError},
    claims,
    claims::{
        Absent, AccessTokenClaims, Acr, Actor, Amr, Audience, AuthenticationClaims,
        BACKCHANNEL_LOGOUT_EVENT, Confirmation, IdTokenClaims, LogoutEvents, LogoutTokenClaims,
        ProfileClaims, UserInfo,
    },
    constants,
    jwt::{Algorithm, Claim, Jwk, JwkSet, KeyId, KeyOperation, KeyType, KeyUse, TokenType},
    metadata,
    metadata::ServerMetadata,
    oauth::{
        ClientAuthMethod, CodeChallengeMethod, ErrorCode, ErrorDescription, ErrorResponse,
        GrantType, Prompt, ResponseMode, ResponseType, Scope, Scopes, SubjectType,
    },
    sdk::{ErrorCategory, ErrorReason, SdkError},
    step_up::StepUpRequirement,
};
use crate::{ClientId, DeliveryId, SessionId, TokenId};
use chrono::TimeDelta;
use std::{collections::BTreeSet, fs, path::PathBuf, time::Duration};
use ts_rs::{Config, TS};

/// Opens the generated file.
const HEADER: &str = "\
// Generated from sourceorigin_common::protocol by `just sdk-types`. Do not edit.
// The wire types auth and every SDK share; docs/dev/architecture/sdk.md describes them.
";

/// Implements `TS` for a type declared in the generated file.
macro_rules! declared {
    ($name:ty, $inline:expr) => {
        impl TS for $name {
            type WithoutGenerics = Self;
            type OptionInnerType = Self;
            fn name(_: &Config) -> String {
                stringify!($name).to_owned()
            }
            fn inline(_: &Config) -> String {
                $inline
            }
            fn decl(cfg: &Config) -> String {
                format!("type {} = {};", Self::name(cfg), Self::inline(cfg))
            }
            fn output_path() -> Option<PathBuf> {
                Some(PathBuf::from("protocol.ts"))
            }
        }
    };
}

/// A closed vocabulary, declared with the list of its spellings.
trait Closed: TS {
    /// The list's name.
    const LIST: &'static str;
    /// Every spelling, in declaration order.
    fn spellings() -> impl Iterator<Item = &'static str>;
}

/// Declares closed vocabularies as the union of their spellings, each with the
/// name of its list.
macro_rules! closed {
    ($($name:ident as $list:ident),+ $(,)?) => {$(
        declared!($name, union($name::spellings()));
        impl Closed for $name {
            const LIST: &'static str = stringify!($list);
            fn spellings() -> impl Iterator<Item = &'static str> {
                $name::ALL.iter().map(|value| value.as_str())
            }
        }
    )+};
}

/// Declares open vocabularies as their known spellings or any other string, which
/// keeps the known ones in editors' completions.
macro_rules! open {
    ($($name:ident),+ $(,)?) => {$(
        declared!(
            $name,
            format!(
                "{} | (string & Record<never, never>)",
                union($name::KNOWN.iter().map(|value| value.as_str()))
            )
        );
    )+};
}

/// Writes types as a built-in TypeScript type, inline: validated strings as
/// `string`, and a claim that must be absent as `never`.
macro_rules! builtin {
    ($builtin:literal => $($name:ty),+ $(,)?) => {$(
        impl TS for $name {
            type WithoutGenerics = Self;
            type OptionInnerType = Self;
            fn name(_: &Config) -> String {
                $builtin.to_owned()
            }
            fn inline(cfg: &Config) -> String {
                Self::name(cfg)
            }
        }
    )+};
}

closed!(
    Amr as AMR,
    Acr as ACR,
    TokenType as TOKEN_TYPES,
    Claim as CLAIMS,
    Algorithm as ALGORITHMS,
    KeyType as KEY_TYPES,
    KeyUse as KEY_USES,
    GrantType as GRANT_TYPES,
    ResponseType as RESPONSE_TYPES,
    ResponseMode as RESPONSE_MODES,
    ClientAuthMethod as CLIENT_AUTH_METHODS,
    SubjectType as SUBJECT_TYPES,
    CodeChallengeMethod as CODE_CHALLENGE_METHODS,
    Prompt as PROMPTS,
    Scope as SCOPES,
    ErrorCategory as ERROR_CATEGORIES,
    ErrorReason as ERROR_REASONS,
);
open!(ErrorCode, BearerError, KeyOperation);
builtin!(
    "string" =>
    ClientId,
    SessionId,
    TokenId,
    DeliveryId,
    KeyId,
    ErrorDescription,
    Scopes
);
declared!(
    LogoutEvents,
    format!("{{ {BACKCHANNEL_LOGOUT_EVENT:?}: Record<string, unknown> }}")
);
builtin!("never" => Absent);

/// A constant `sdk_constants!` declares, as TypeScript writes it.
pub(super) trait Literal {
    /// Its TypeScript type, or none for the literal's own, as strings keep.
    fn ty(&self) -> Option<&'static str>;
    /// Its value.
    fn value(&self) -> String;
    /// What its docs add: durations are in milliseconds.
    fn unit(&self) -> &'static str {
        ""
    }
}

/// Writes integers as `number`s.
macro_rules! number {
    ($($type:ty),+) => {$(
        impl Literal for $type {
            fn ty(&self) -> Option<&'static str> {
                Some("number")
            }
            fn value(&self) -> String {
                self.to_string()
            }
        }
    )+};
}

/// Writes durations as `number`s of milliseconds.
macro_rules! milliseconds {
    ($($type:ty => $millis:ident),+) => {$(
        impl Literal for $type {
            fn ty(&self) -> Option<&'static str> {
                Some("number")
            }
            fn value(&self) -> String {
                self.$millis().to_string()
            }
            fn unit(&self) -> &'static str {
                "\nIn milliseconds."
            }
        }
    )+};
}

number!(u8, usize);
milliseconds!(TimeDelta => num_milliseconds, Duration => as_millis);

impl Literal for &str {
    fn ty(&self) -> Option<&'static str> {
        None
    }
    fn value(&self) -> String {
        format!("{self:?}")
    }
}

/// `"a" | "b"`, from spellings that need no escaping.
fn union<'a>(values: impl Iterator<Item = &'a str>) -> String {
    values
        .map(|value| format!("{value:?}"))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The generated file, built from declarations in order.
struct Declarations {
    /// Options for ts-rs.
    cfg: Config,
    /// The file so far.
    text: String,
    /// Names declared so far.
    declared: BTreeSet<String>,
    /// Names each declaration refers to.
    referenced: BTreeSet<String>,
}

impl Declarations {
    /// Starts the file.
    fn new() -> Self {
        Self {
            cfg: Config::new().with_large_int("number"),
            text: HEADER.to_owned(),
            declared: BTreeSet::new(),
            referenced: BTreeSet::new(),
        }
    }

    /// Appends `T`'s declaration, with its docs.
    fn add<T: TS + 'static>(mut self) -> Self {
        self.text.push('\n');
        self.text.push_str(&T::docs().unwrap_or_default());
        self.text.push_str("export ");
        self.text.push_str(&T::decl(&self.cfg));
        self.text.push('\n');
        self.declared.insert(T::ident(&self.cfg));
        self.referenced.extend(
            T::dependencies(&self.cfg)
                .into_iter()
                .map(|dependency| dependency.ts_name),
        );
        self
    }

    /// Appends a closed vocabulary's declaration and the list of its spellings.
    fn closed<T: Closed + 'static>(self) -> Self {
        let declarations = self.add::<T>();
        let name = T::ident(&declarations.cfg);
        let spellings: Vec<_> = T::spellings().map(|value| format!("{value:?}")).collect();
        declarations.constant(
            T::LIST,
            &format!("Every `{name}` this version knows."),
            Some(&format!("readonly {name}[]")),
            &format!("[{}]", spellings.join(", ")),
        )
    }

    /// Appends a constant of type `ty`, or of its literal's, with its docs.
    fn constant(mut self, name: &str, docs: &str, ty: Option<&str>, value: &str) -> Self {
        self.text.push_str("\n/**\n");
        for line in docs.lines() {
            self.text.push_str(" * ");
            self.text.push_str(line.trim());
            self.text.push('\n');
        }
        let ty = ty.map(|ty| format!(": {ty}")).unwrap_or_default();
        for part in [" */\nexport const ", name, &ty, " = ", value, ";\n"] {
            self.text.push_str(part);
        }
        self
    }

    /// Appends constants `sdk_constants!` declared.
    fn constants(self, constants: Vec<(&str, &str, &dyn Literal)>) -> Self {
        constants
            .into_iter()
            .fold(self, |declarations, (name, docs, value)| {
                declarations.constant(
                    name,
                    &(docs.to_owned() + value.unit()),
                    value.ty(),
                    &value.value(),
                )
            })
    }

    /// The file, once every referenced type is declared in it. Drops the docs' Rust
    /// examples, the colon that introduced a final one and rustdoc's link brackets,
    /// and ts-rs's trailing spaces.
    fn finish(self) -> String {
        let missing: Vec<_> = self.referenced.difference(&self.declared).collect();
        assert!(missing.is_empty(), "declare {missing:?} too");
        let mut example = false;
        let mut lines: Vec<String> = Vec::new();
        for line in self.text.lines().map(str::trim_end) {
            let fence = line
                .trim_start()
                .strip_prefix('*')
                .is_some_and(|doc| doc.trim().starts_with("```"));
            example ^= fence;
            if example || fence {
                continue;
            }
            if line == " */" {
                // An example that ended a doc leaves an empty line before its end.
                if lines.last().is_some_and(|last| last == " *") {
                    lines.pop();
                }
                if let Some(last) = lines.last_mut().filter(|last| last.ends_with(':')) {
                    last.pop();
                    last.push('.');
                }
            }
            if line.starts_with(" *") {
                lines.push(line.replace("[`", "`").replace("`]", "`"));
            } else {
                lines.push(line.to_owned());
            }
        }
        lines.join("\n") + "\n"
    }
}

/// Every type the TypeScript SDK reads, grouped as in `docs/dev/architecture/sdk.md`.
fn generate() -> String {
    Declarations::new()
        // claims
        .add::<AccessTokenClaims>()
        .add::<IdTokenClaims>()
        .add::<LogoutTokenClaims>()
        .add::<LogoutEvents>()
        .add::<UserInfo>()
        .add::<AuthenticationClaims>()
        .add::<ProfileClaims>()
        .add::<Audience>()
        .add::<Actor>()
        .add::<Confirmation>()
        .closed::<Amr>()
        .closed::<Acr>()
        .constants(claims::typescript())
        // metadata
        .add::<ServerMetadata>()
        .constants(metadata::typescript())
        // step_up
        .add::<StepUpRequirement>()
        // jwt
        .closed::<TokenType>()
        .closed::<Claim>()
        .closed::<Algorithm>()
        .add::<JwkSet>()
        .add::<Jwk>()
        .closed::<KeyType>()
        .closed::<KeyUse>()
        .add::<KeyOperation>()
        // oauth
        .closed::<GrantType>()
        .closed::<ResponseType>()
        .closed::<ResponseMode>()
        .closed::<ClientAuthMethod>()
        .closed::<SubjectType>()
        .closed::<CodeChallengeMethod>()
        .closed::<Prompt>()
        .closed::<Scope>()
        .add::<ErrorCode>()
        .add::<ErrorResponse>()
        // challenge
        .add::<BearerChallenge>()
        .add::<BearerError>()
        // sdk
        .add::<SdkError>()
        .closed::<ErrorCategory>()
        .closed::<ErrorReason>()
        // constants
        .constants(constants::typescript())
        .finish()
}

/// The committed declarations match the Rust types.
#[test]
fn typescript_declarations_are_current() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/auth/src/protocol.ts");
    let generated = generate();
    if std::env::var_os("UPDATE_SDK_TYPES").is_some() {
        fs::write(&path, generated).expect("write the declarations");
        return;
    }
    assert!(
        fs::read_to_string(&path).ok().as_ref() == Some(&generated),
        "{} is stale; run `just sdk-types`",
        path.display()
    );
}

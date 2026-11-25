//! What auth's own pages report about themselves: how fast they loaded and the
//! script errors browsers hit, which the vitals script (`assets/vitals.js`)
//! posts once per page view, recorded as operational signals. See
//! `docs/dev/architecture/analytics.md`.

use crate::{EmbeddedAssets, routes};
use axum::{Json, Router, extract::DefaultBodyLimit, http::StatusCode, routing::post};
use serde::{Deserialize, Deserializer};
use sourceorigin_telemetry::analytics::Signal;
use std::borrow::Cow;

/// Bound on a report's body: its path, five timings and five errors take a few
/// hundred bytes.
const MAX_REPORT_BYTES: usize = 4 * 1024;
/// The errors one report records at most, so one page can't flood the table.
const MAX_ERRORS: usize = 5;
/// The longest timing recorded, in milliseconds: a minute, past which a page
/// isn't loading but waiting.
const MAX_MS: u32 = 60_000;
/// The largest layout shift recorded, in thousandths: 10, far past the 0.25
/// that browsers call poor.
const MAX_SHIFT: u32 = 10_000;

sourceorigin_telemetry::signal! {
    /// How fast one view of one of auth's pages loaded, as its browser timed
    /// it. It names only the page's route template, never its path, and no
    /// account or browser, so it says how auth runs, like `requests`, and needs
    /// no consent.
    pub(crate) struct PageTimed in "page_timings", kept 30 days, ordered by (route) {
        /// The page's route template, such as `/developers/clients/{id}`.
        pub(crate) route: &'static str,
        /// Until the response's first byte, in milliseconds.
        pub(crate) ttfb_ms: Option<u32>,
        /// Until the first contentful paint, in milliseconds.
        pub(crate) fcp_ms: Option<u32>,
        /// Until the largest contentful paint, in milliseconds.
        pub(crate) lcp_ms: Option<u32>,
        /// The longest interaction, in milliseconds: an approximation of
        /// Interaction to Next Paint. None without an interaction.
        pub(crate) inp_ms: Option<u32>,
        /// The cumulative layout shift, in thousandths.
        pub(crate) cls: Option<u32>,
    }
}

sourceorigin_telemetry::signal! {
    /// An error no script caught on one of auth's pages, as its browser
    /// reported it: only the error's kind and which of auth's scripts threw it,
    /// never its message or stack, which can quote what the page shows. Like
    /// `page_timings`, it names no account or browser, and needs no consent.
    pub(crate) struct ScriptFailed in "script_errors", kept 30 days, ordered by (name, script) {
        /// The page's route template.
        pub(crate) route: &'static str,
        /// The error's constructor, such as `TypeError`, or `other`.
        pub(crate) name: &'static str,
        /// Which of auth's scripts threw it, such as `passkey`, or `other`.
        pub(crate) script: &'static str,
    }
}

sourceorigin_common::str_enum! {
    /// The kind of an error a page's script didn't catch, by its constructor:
    /// JavaScript's own errors, and the DOM's.
    pub(crate) enum ScriptErrorName {
        /// A plain `Error`.
        Error = "Error",
        /// A value used as a type it isn't, such as calling `undefined`.
        TypeError = "TypeError",
        /// A name nothing declares.
        ReferenceError = "ReferenceError",
        /// Code that doesn't parse, such as in `JSON.parse`.
        SyntaxError = "SyntaxError",
        /// A number out of its range.
        RangeError = "RangeError",
        /// A malformed URI component.
        UriError = "URIError",
        /// Several errors at once, such as from `Promise.any`.
        AggregateError = "AggregateError",
        /// A failure of a browser API, such as a cancelled passkey prompt.
        DomException = "DOMException",
        /// Any other value thrown, or an error class of a script's own.
        Other = "other",
    } else Other
}

/// What the vitals script posts: the page's path, the timings it measured, as
/// whole numbers that are clamped as they are read, and the errors it noted.
#[derive(Debug, Deserialize)]
struct PageReport {
    /// The page's `location.pathname`.
    path: String,
    /// Until the response's first byte.
    ttfb: Option<Clamped<MAX_MS>>,
    /// Until the first contentful paint.
    fcp: Option<Clamped<MAX_MS>>,
    /// Until the largest contentful paint.
    lcp: Option<Clamped<MAX_MS>>,
    /// The longest interaction.
    inp: Option<Clamped<MAX_MS>>,
    /// The cumulative layout shift, in thousandths.
    cls: Option<Clamped<MAX_SHIFT>>,
    /// The errors no script caught, the first few kept.
    #[serde(default)]
    errors: Vec<ScriptError>,
}

/// An error the vitals script noted.
#[derive(Debug, Deserialize)]
struct ScriptError {
    /// Its kind.
    name: ScriptErrorName,
    /// The script that threw it.
    script: ScriptName,
}

/// A whole number of at most `MAX`, from any a report holds: a larger one reads
/// as `MAX`. A negative or fractional one, which the script never sends, fails
/// the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Clamped<const MAX: u32>(u32);

impl<const MAX: u32> Clamped<MAX> {
    /// The number.
    const fn get(self) -> u32 {
        self.0
    }
}

impl<'de, const MAX: u32> Deserialize<'de> for Clamped<MAX> {
    /// Reads a whole number, clamped to `MAX`.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = u64::deserialize(deserializer)?;
        Ok(Self(
            u32::try_from(value).map_or(MAX, |value| value.min(MAX)),
        ))
    }
}

/// One of the scripts auth's pages load, by its name, such as `passkey`, or
/// `other` for any name that isn't one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScriptName(&'static str);

impl<'de> Deserialize<'de> for ScriptName {
    /// Reads a name, keeping it only if one of [`EmbeddedAssets::SCRIPTS`] has it.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = Cow::<str>::deserialize(deserializer)?;
        Ok(Self(
            EmbeddedAssets::SCRIPTS
                .iter()
                .map(sourceorigin_ui::Asset::stem)
                .find(|stem| *stem == name)
                .unwrap_or("other"),
        ))
    }
}

impl PageReport {
    /// Records the report's timings and its first [`MAX_ERRORS`] errors, against
    /// the route its path is a page of, or nothing for a path no page has: only
    /// route templates are ever recorded.
    fn record(self) {
        let Some(route) = routes::page::template_of(&self.path) else {
            return;
        };
        PageTimed {
            route,
            ttfb_ms: self.ttfb.map(Clamped::get),
            fcp_ms: self.fcp.map(Clamped::get),
            lcp_ms: self.lcp.map(Clamped::get),
            inp_ms: self.inp.map(Clamped::get),
            cls: self.cls.map(Clamped::get),
        }
        .record();
        for ScriptError {
            name,
            script: ScriptName(script),
        } in self.errors.into_iter().take(MAX_ERRORS)
        {
            ScriptFailed {
                route,
                name: name.as_str(),
                script,
            }
            .record();
        }
    }
}

/// The route auth's pages post their reports to. It needs no state: it reads
/// no session and queries no database, and the security layer takes a report
/// only from auth's own pages, with the CSRF header, within its own budget.
pub(crate) struct VitalsApplication;

impl VitalsApplication {
    /// Returns the report route, under a body limit of its own.
    pub(crate) fn routes() -> Router {
        Router::new().route(
            routes::telemetry::PAGE,
            post(Self::report).layer(DefaultBodyLimit::max(MAX_REPORT_BYTES)),
        )
    }

    /// Records `report`, and answers `204` whether or not it named a page.
    fn report(Json(report): Json<PageReport>) -> std::future::Ready<StatusCode> {
        report.record();
        std::future::ready(StatusCode::NO_CONTENT)
    }
}

#[cfg(test)]
mod tests;

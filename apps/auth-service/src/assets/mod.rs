//! The stylesheet, typefaces and scripts of auth's pages, and the admin
//! dashboard's script, embedded under paths that never serve other content, so
//! browsers and shared caches keep them forever.

use crate::security::{ApiError, CacheClass, Cached};
use axum::{
    http::header,
    response::{IntoResponse, Response},
};
use sourceorigin_ui::Asset;

/// The files of auth's pages, embedded in this executable.
pub(crate) struct EmbeddedAssets;
impl EmbeddedAssets {
    /// The stylesheet of auth's pages, `sourceorigin-ui`'s.
    pub(crate) const STYLESHEET: Asset = sourceorigin_ui::STYLESHEET;
    /// The passkey script, which runs the pages' passkey ceremonies.
    pub(crate) const SCRIPT: Asset =
        Asset::script("passkey", include_bytes!("../../assets/passkey.js"));
    /// The picture script, which lets the profile's picture form choose its crop.
    pub(crate) const PICTURE_SCRIPT: Asset =
        Asset::script("picture", include_bytes!("../../assets/picture.js"));
    /// The search script, which ranks the top bar's results as the user types and
    /// refreshes a live form's results; auth's pages and the dashboard's both
    /// load it.
    pub(crate) const SEARCH_SCRIPT: Asset =
        Asset::script("search", include_bytes!("../../assets/search.js"));
    /// The copy script, whose buttons, and the values beside them, copy a value
    /// to the clipboard; auth's pages and the dashboard's both load it.
    pub(crate) const COPY_SCRIPT: Asset =
        Asset::script("copy", include_bytes!("../../assets/copy.js"));
    /// The client form script, which switches a new client's scopes and refresh
    /// tokens to the defaults of the kind chosen; auth's pages and the
    /// dashboard's both load it.
    pub(crate) const CLIENT_FORM_SCRIPT: Asset =
        Asset::script("client-form", include_bytes!("../../assets/client-form.js"));
    /// The vitals script, which reports how fast each of auth's pages loaded,
    /// and the script errors it hit, to [`routes::telemetry::PAGE`](crate::routes::telemetry::PAGE).
    pub(crate) const VITALS_SCRIPT: Asset =
        Asset::script("vitals", include_bytes!("../../assets/vitals.js"));
    /// The scripts every page of auth's own loads, the vitals script first, so
    /// it hears the others' errors.
    pub(crate) const SCRIPTS: [Asset; 6] = [
        Self::VITALS_SCRIPT,
        Self::SCRIPT,
        Self::PICTURE_SCRIPT,
        Self::SEARCH_SCRIPT,
        Self::COPY_SCRIPT,
        Self::CLIENT_FORM_SCRIPT,
    ];
    /// The admin dashboard's script, which confirms it with a passkey in a
    /// dialog on its page. Auth's own pages never load it.
    pub(crate) const DASHBOARD_SCRIPT: Asset =
        Asset::script("dashboard", include_bytes!("../../assets/dashboard.js"));
    /// The scripts every dashboard page loads.
    pub(crate) const DASHBOARD_SCRIPTS: [Asset; 4] = [
        Self::DASHBOARD_SCRIPT,
        Self::SEARCH_SCRIPT,
        Self::COPY_SCRIPT,
        Self::CLIENT_FORM_SCRIPT,
    ];
    /// The files the admin dashboard serves too: the stylesheet and the
    /// typefaces it links.
    pub(crate) const SHARED: [Asset; 4] = [
        Self::STYLESHEET,
        sourceorigin_ui::FONTS[0],
        sourceorigin_ui::FONTS[1],
        sourceorigin_ui::FONTS[2],
    ];

    /// Serves the embedded file at exactly `path`, or a `404` in the JSON shape
    /// of other browser errors. A file's path changes with its content, or its
    /// version, so nothing needs revalidating.
    pub(crate) fn serve(path: &str) -> Result<Response, ApiError> {
        let asset = Self::SHARED
            .into_iter()
            .chain(Self::SCRIPTS)
            .chain(Self::DASHBOARD_SCRIPTS)
            .find(|asset| asset.path() == path)
            .ok_or_else(|| ApiError::not_found("There is no page at this address."))?;
        let headers = [(header::CONTENT_TYPE, asset.content_type())];
        Ok(Cached(CacheClass::Immutable, (headers, asset.bytes())).into_response())
    }
}

/// Focused coverage of embedded file serving.
#[cfg(test)]
mod tests;

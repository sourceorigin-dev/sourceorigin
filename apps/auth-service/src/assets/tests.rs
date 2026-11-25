//! Focused coverage of embedded file serving.

use super::*;

/// The pages' stylesheet, typefaces and scripts are served at the paths
/// naming their content or version, as their own type, so they are cached
/// forever.
#[test]
fn the_pages_files_are_immutable() {
    for (asset, content_type) in [
        (EmbeddedAssets::STYLESHEET, "text/css; charset=utf-8"),
        (EmbeddedAssets::SCRIPT, "text/javascript; charset=utf-8"),
        (
            EmbeddedAssets::PICTURE_SCRIPT,
            "text/javascript; charset=utf-8",
        ),
        (
            EmbeddedAssets::DASHBOARD_SCRIPT,
            "text/javascript; charset=utf-8",
        ),
        (
            EmbeddedAssets::SEARCH_SCRIPT,
            "text/javascript; charset=utf-8",
        ),
        (
            EmbeddedAssets::COPY_SCRIPT,
            "text/javascript; charset=utf-8",
        ),
        (
            EmbeddedAssets::CLIENT_FORM_SCRIPT,
            "text/javascript; charset=utf-8",
        ),
        (
            EmbeddedAssets::VITALS_SCRIPT,
            "text/javascript; charset=utf-8",
        ),
        (sourceorigin_ui::FONTS[0], "font/woff2"),
    ] {
        let response = EmbeddedAssets::serve(&asset.path()).expect("found");
        assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
        assert_eq!(
            response.extensions().get::<CacheClass>(),
            Some(&CacheClass::Immutable)
        );
    }
}

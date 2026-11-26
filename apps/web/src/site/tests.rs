//! The site as a browser sees it.
use super::*;
use axum::body::{Body, to_bytes};
use tower::ServiceExt;

/// Auth's origin in tests.
const AUTH: &str = "https://auth.sourceorigin.dev";

/// `path`'s answer.
async fn visit(path: &str) -> Response {
    Site::new(AUTH.parse().expect("auth origin"))
        .router()
        .oneshot(
            axum::http::Request::get(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("answer")
}

/// The homepage and the policies are pages search engines index at their own
/// path, which load nothing but the stylesheet, and whose footer links the
/// policies here.
#[tokio::test]
async fn serves_its_pages() {
    for (page, heading) in [
        (Page::Home, "One passkey for every SourceOrigin app."),
        (Page::Privacy, "Privacy policy"),
        (Page::Terms, "Terms of service"),
    ] {
        let response = visit(page.path()).await;
        assert_eq!(response.status(), StatusCode::OK, "{page:?}");
        assert_eq!(response.headers()[header::CONTENT_SECURITY_POLICY], POLICY);
        assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
        let body = to_bytes(response.into_body(), 1 << 20).await.expect("body");
        let html = String::from_utf8(body.to_vec()).expect("UTF-8");
        assert!(html.contains(heading), "{page:?}");
        assert!(!html.contains("noindex"), "{page:?}");
        let canonical = format!(r#"<link rel="canonical" href="{}">"#, page.path());
        assert!(html.contains(&canonical), "{page:?}");
        assert!(html.contains(r#"href="/privacy""#), "{page:?}");
        assert!(html.contains(&STYLESHEET.path()), "{page:?}");
    }
    for asset in ASSETS {
        let served = visit(&asset.path()).await;
        assert_eq!(served.status(), StatusCode::OK, "{}", asset.path());
    }
}

/// Every other path, a sent settings link's included, goes on to auth with its
/// query, and nobody caches that. The redirect is isolated as a page is.
#[tokio::test]
async fn sends_every_other_path_to_auth() {
    let response = visit("/settings/emails/verify?remove=1").await;
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers()[header::LOCATION],
        "https://auth.sourceorigin.dev/settings/emails/verify?remove=1"
    );
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "no-store, no-transform"
    );
    let headers = response.headers();
    assert_eq!(headers[header::CONTENT_SECURITY_POLICY], POLICY);
    assert_eq!(headers["cross-origin-opener-policy"], "same-origin");
    assert_eq!(headers["cross-origin-resource-policy"], "same-origin");
    assert_eq!(
        headers["permissions-policy"],
        sourceorigin_common::protocol::isolation::PERMISSIONS_POLICY
    );
}

/// Every answer forbids transforming its body, so the edge never rewrites the
/// policies' email links into a script the Content-Security-Policy blocks.
#[tokio::test]
async fn no_answer_may_be_transformed() {
    let paths = Page::ALL
        .into_iter()
        .map(|page| page.path().to_owned())
        .chain(ASSETS.map(|asset| asset.path()))
        .chain([HEALTH.to_owned(), "/settings".to_owned()]);
    for path in paths {
        let response = visit(&path).await;
        let cache = response.headers()[header::CACHE_CONTROL]
            .to_str()
            .expect("ASCII");
        assert!(
            cache
                .split(", ")
                .any(|directive| directive == "no-transform"),
            "{path}: {cache}"
        );
    }
}

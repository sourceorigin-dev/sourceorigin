//! How a trace's spans are ordered.

use super::*;

/// A span `span` under `parent`, started at `start_us`.
fn span(span: &str, parent: &str, start_us: i64) -> Span {
    Span {
        start_us,
        id: span.into(),
        parent: parent.into(),
        service: "auth-service".into(),
        name: span.into(),
        status: "unset".into(),
        duration_us: 1,
        attributes: BTreeMap::new(),
    }
}

/// Each span follows its parent and its earlier siblings, one deeper; a span
/// whose parent isn't in the trace is a root, and so is the first of spans no
/// root leads to, so every span shows once.
#[test]
fn spans_follow_their_parents() {
    let spans = vec![
        span("root", "", 0),
        span("first", "root", 1),
        span("second", "root", 2),
        span("nested", "first", 3),
        span("orphan", "elsewhere", 4),
        span("looped", "looping", 5),
        span("looping", "looped", 6),
        span("itself", "itself", 7),
    ];
    let order: Vec<(usize, String)> = Trace::tree(spans)
        .into_iter()
        .map(|(depth, span)| (depth, span.id))
        .collect();
    assert_eq!(
        order,
        [
            (0, "root".to_owned()),
            (1, "first".to_owned()),
            (2, "nested".to_owned()),
            (1, "second".to_owned()),
            (0, "orphan".to_owned()),
            (0, "looped".to_owned()),
            (1, "looping".to_owned()),
            (0, "itself".to_owned()),
        ]
    );
}

//! How the Analytics page's periods are filled in.

use super::*;

/// A chart has one row per hour, oldest first, ending with the current hour,
/// whichever hours had rows; rows older than its first hour are left out.
#[test]
fn every_hour_has_a_row() {
    let at = |at: &str| at.parse::<DateTime<Utc>>().expect("time");
    let hour = |text: &str| at(text).timestamp();
    let now = at("2026-11-24T10:30:00Z");
    assert_eq!(HourOfRequests::since(now), hour("2026-11-23T11:00:00Z"));
    let rows = vec![
        HourOfRequests {
            hour: hour("2026-11-23T09:00:00Z"),
            requests: 1,
            errors: 0,
        },
        HourOfRequests {
            hour: hour("2026-11-24T08:00:00Z"),
            requests: 5,
            errors: 1,
        },
    ];
    let filled = HourOfRequests::every(rows, now);
    assert_eq!(filled.len(), 24);
    let counts: Vec<(i64, u32)> = filled[21..]
        .iter()
        .map(|row| (row.hour, row.requests))
        .collect();
    assert_eq!(
        counts,
        [
            (hour("2026-11-24T08:00:00Z"), 5),
            (hour("2026-11-24T09:00:00Z"), 0),
            (hour("2026-11-24T10:00:00Z"), 0),
        ]
    );
    assert_eq!(filled.iter().map(|row| row.requests).sum::<u32>(), 5);
}

/// Transports this release doesn't know add up into one kind, and the kinds
/// stay most used first.
#[test]
fn unknown_transports_add_up() {
    let kind = |transport, count| PasskeyKind {
        synced: false,
        transport,
        count,
    };
    let merged = PasskeyKind::merged(vec![
        kind(PasskeyTransport::Internal, 10),
        kind(PasskeyTransport::Unknown, 6),
        kind(PasskeyTransport::Unknown, 6),
    ]);
    assert_eq!(
        merged,
        [
            kind(PasskeyTransport::Unknown, 12),
            kind(PasskeyTransport::Internal, 10),
        ]
    );
}

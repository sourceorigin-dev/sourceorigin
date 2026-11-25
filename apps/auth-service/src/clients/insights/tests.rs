//! Which counts an app's owner sees, and when.

use super::*;

/// The noise of a test key.
fn noise() -> Noise {
    Noise::new(Secret::derive(
        b"a test key of at least 32 bytes!",
        b"noise",
    ))
}

/// The client the tests count for.
fn client() -> ClientId {
    "c_insights".parse().expect("client")
}

/// One day's count of `step`.
const fn day(day: i64, step: AuthorizationStepKind, count: u32) -> DayOfStep {
    DayOfStep { day, step, count }
}

/// What the owner sees of `step` among `steps`.
fn seen(
    steps: &[(AuthorizationStepKind, Floored)],
    step: AuthorizationStepKind,
) -> Option<Floored> {
    steps
        .iter()
        .find(|(seen, _)| *seen == step)
        .map(|(_, count)| *count)
}

/// A day's offset is under the floor, the same each time, and differs from
/// day to day.
#[test]
fn each_day_draws_once() {
    let (noise, client) = (noise(), client());
    let offsets: Vec<u32> = (0..30).map(|n| noise.offset(&client, n * DAY)).collect();
    assert!(offsets.iter().all(|offset| *offset < FLOOR), "{offsets:?}");
    assert!(
        offsets.iter().any(|offset| *offset != offsets[0]),
        "{offsets:?}"
    );
    assert_eq!(noise.offset(&client, 0), offsets[0]);
}

/// Each day rounds on its own, to a multiple of the floor, and the days add
/// up; a day nothing reached never counts, whatever its offset.
#[test]
fn days_round_then_add_up() {
    let (noise, client) = (noise(), client());
    let step = AuthorizationStepKind::Authorized;
    let rounded = |n: i64, count: u32| {
        let moved = count + noise.offset(&client, n * DAY);
        moved - moved % FLOOR
    };
    let days: Vec<DayOfStep> = (0..30).map(|n| day(n * DAY, step, 3)).collect();
    let total: u32 = (0..30).map(|n| rounded(n, 3)).sum();
    assert_eq!(
        seen(&noise.floored(&client, &days), step),
        Some(Floored::of(total))
    );
    assert!((60..=120).contains(&total), "{total}");
    let none: Vec<DayOfStep> = (0..30).map(|n| day(n * DAY, step, 0)).collect();
    assert_eq!(
        seen(&noise.floored(&client, &none), step),
        Some(Floored::Few)
    );
}

/// A step another release wrote shows only once it is no longer few.
#[test]
fn other_steps_show_only_once_counted() {
    let (noise, client) = (noise(), client());
    let other = AuthorizationStepKind::Other;
    assert_eq!(seen(&noise.floored(&client, &[]), other), None);
    assert_eq!(
        seen(&noise.floored(&client, &[day(0, other, 0)]), other),
        None
    );
    let counted = noise.floored(&client, &[day(0, other, FLOOR)]);
    assert!(matches!(seen(&counted, other), Some(Floored::About(_))));
}

/// A day's counts show once twice the product interval has passed since it
/// ended, so its last hour's rows are in.
#[test]
fn a_day_shows_once_its_rows_are_in() {
    let at = |text: &str| text.parse::<DateTime<Utc>>().expect("time");
    let until = |text: &str| AppInsights::until(at(text));
    let start = |text: &str| at(text).timestamp();
    assert_eq!(until("2026-11-25T01:30:00Z"), start("2026-11-24T00:00:00Z"));
    assert_eq!(until("2026-11-25T02:30:00Z"), start("2026-11-25T00:00:00Z"));
}

/// A day's steps move together, so a step reached less often that day never
/// shows more than one reached more often.
#[test]
fn a_days_steps_keep_their_order() {
    let (noise, client) = (noise(), client());
    for n in 0..30 {
        let days = [
            day(n * DAY, AuthorizationStepKind::Requested, 7),
            day(n * DAY, AuthorizationStepKind::Authorized, 6),
        ];
        let steps = noise.floored(&client, &days);
        let total = |step| match seen(&steps, step) {
            Some(Floored::About(total)) => total,
            _ => 0,
        };
        assert!(
            total(AuthorizationStepKind::Authorized) <= total(AuthorizationStepKind::Requested),
            "{steps:?}"
        );
    }
}

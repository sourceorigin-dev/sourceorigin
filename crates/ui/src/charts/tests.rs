//! How charts scale and write their numbers.

use super::*;

/// The value axis rounds up to a few round steps, whatever the largest value.
#[test]
fn scales_round_up_in_round_steps() {
    assert_eq!(Scale::of(0.0).ticks, [0.0, 1.0]);
    assert_eq!(Scale::of(3.0).ticks, [0.0, 1.0, 2.0, 3.0]);
    assert_eq!(Scale::of(1_284.0).ticks, [0.0, 500.0, 1_000.0, 1_500.0]);
}

/// Numbers are grouped below ten thousand, and compact above.
#[test]
fn numbers_are_written_compactly() {
    assert_eq!(compact(1_284.0), "1,284");
    assert_eq!(compact(12_900.0), "12.9K");
    assert_eq!(compact(4_200_000.0), "4.2M");
    assert_eq!(compact(20_000.0), "20K");
    assert_eq!(compact(2.5), "2.5");
}

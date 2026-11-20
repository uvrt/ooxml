//! Exact arithmetic: floors of negative rationals, exact division that refuses to round.

use ooxml_core::units::{exact_div, floor, q};

#[test]
fn floor_rounds_down() {
    assert_eq!(floor(q(7, 2)), 3);
    assert_eq!(floor(q(-7, 2)), -4);
    assert_eq!(floor(q(6, 3)), 2);
}

#[test]
fn exact_div_divides() {
    assert_eq!(exact_div(4096 * 3, 4096), 3);
}

#[test]
#[should_panic(expected = "inexact")]
fn exact_div_refuses_to_round() {
    exact_div(7, 2);
}

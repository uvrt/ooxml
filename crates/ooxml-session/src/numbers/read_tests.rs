//! ooxml-edit's `test_tools_numbers.py`, "reading numbers", ported.

use super::{find_numbers, shown, NumberRules};
use ooxml_numfmt::Locale;

fn nl_be() -> Locale {
    Locale {
        decimal_separator: ',',
        thousands_separator: '.',
        ..Locale::default()
    }
}

fn rules(whole_numbers: bool) -> NumberRules {
    NumberRules {
        whole_numbers,
        ..NumberRules::default()
    }
}

/// `(text, value, decimals, percent, negative)` of each number read, and those left aside.
type Read = (Vec<(String, String, usize, Option<u32>, bool)>, Vec<(String, usize)>);

fn read_with(text: &str, locale: &Locale, rules: &NumberRules) -> Read {
    let (found, aside) = find_numbers(text, locale, rules, &[]).unwrap();
    (
        found
            .into_iter()
            .map(|f| {
                (
                    f.text,
                    format!("{}", f.value.to_f64()),
                    f.decimals,
                    f.percent,
                    f.negative,
                )
            })
            .collect(),
        aside.into_iter().collect(),
    )
}

fn texts(read: &Read) -> Vec<&str> {
    read.0.iter().map(|f| f.0.as_str()).collect()
}

fn aside(pairs: &[(&str, usize)]) -> Vec<(String, usize)> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
}

fn n(
    text: &str,
    value: &str,
    decimals: usize,
    percent: Option<u32>,
    negative: bool,
) -> (String, String, usize, Option<u32>, bool) {
    (text.to_owned(), value.to_owned(), decimals, percent, negative)
}

#[test]
fn decimals_and_percentages_are_read_in_the_locale() {
    let read = read_with(
        "Gemiddeld 2,7/5 voor 1.234,5 klanten (85 %), daling van −2,5.",
        &nl_be(),
        &rules(false),
    );
    assert_eq!(
        read.0,
        vec![
            n("2,7", "2.7", 1, None, false),
            n("1.234,5", "1234.5", 1, None, false),
            n("85 %", "85", 0, Some(100), false),
            n("−2,5", "2.5", 1, None, true),
        ]
    );
    assert_eq!(read.1, aside(&[("whole", 1)]));
    let read = read_with(
        "Rated 2.7/5 by 1,234.5 users; 85% said yes; 0.5‰ left",
        &Locale::default(),
        &rules(false),
    );
    assert_eq!(texts(&read), ["2.7", "1,234.5", "85%", "0.5‰"]);
    assert_eq!(read.0[3].3, Some(1000));
}

#[test]
fn a_space_groups_thousands_only_where_the_locale_does() {
    assert_eq!(
        texts(&read_with("123.4 567.8 and 12.5", &Locale::default(), &rules(false))),
        ["123.4", "567.8", "12.5"]
    );
    let fr = Locale {
        decimal_separator: ',',
        thousands_separator: '\u{202f}',
        ..Locale::default()
    };
    assert_eq!(
        read_with("soit 1\u{202f}234,5 et 2,5", &fr, &rules(false)).0,
        vec![
            n("1\u{202f}234,5", "1234.5", 1, None, false),
            n("2,5", "2.5", 1, None, false)
        ]
    );
}

#[test]
fn the_other_convention_s_decimal_is_read_as_a_reader_would() {
    let read = read_with("Prijs 2.7 euro voor 2.500 stuks", &nl_be(), &rules(false));
    assert_eq!(read.0, vec![n("2.7", "2.7", 1, None, false)]);
    assert_eq!(read.1, aside(&[("whole", 1)]));
    let read = read_with("rated 2,7 here", &Locale::default(), &rules(false));
    assert_eq!(read.0, vec![n("2,7", "2.7", 1, None, false)]);
}

#[test]
fn a_range_is_two_numbers_and_a_sign_only_before_a_number() {
    let read = read_with("between 2.5-3.5 days, down -1.25", &Locale::default(), &rules(false));
    assert_eq!(
        read.0,
        vec![
            n("2.5", "2.5", 1, None, false),
            n("3.5", "3.5", 1, None, false),
            n("-1.25", "1.25", 2, None, true)
        ]
    );
}

#[test]
fn dates_times_years_list_numbers_and_ids_are_left_aside() {
    let read = read_with(
        "On 12/03/2026, 2026-03-12 and 12.03.2026 at 14:05 (or 14.30 uur), 3 March, in 2024: Q3, CO2, 2.1.3",
        &Locale::default(),
        &rules(true),
    );
    assert_eq!(read.1, aside(&[("dates", 12), ("years", 1)]));
    let read = read_with(
        "1. First 2.5\n2.3 Results were 4.25\n(4) last",
        &Locale::default(),
        &rules(false),
    );
    assert_eq!(texts(&read), ["2.5", "4.25"]);
    assert_eq!(read.1, aside(&[("list numbers", 3)]));
    let read = read_with(
        "v2.1 build, 3D view at #4.5, 2nd and 1.5x",
        &Locale::default(),
        &rules(false),
    );
    assert!(read.0.is_empty());
    assert_eq!(read.1, aside(&[("ids", 3)]));
}

#[test]
fn a_number_named_by_the_word_before_it_is_an_id() {
    let read = read_with(
        "see section 4.2, page 12.5 and version 2.1; hoofdstuk 3.4 holds 4.25",
        &Locale::default(),
        &rules(false),
    );
    assert_eq!(texts(&read), ["4.25"]);
    assert_eq!(read.1, aside(&[("ids", 4)]));
}

#[test]
fn an_identifier_s_digits_are_text_not_a_number() {
    let read = read_with(
        "K7-12.3, DOC-004.2 and ab-cd-3.4 hold M.4-2.7, x-1.5 and Lv2-0.8 (NET\u{2011}12.5, SEC\u{2010}9.75)",
        &Locale::default(),
        &rules(true),
    );
    assert!(read.0.is_empty());
    assert_eq!(read.1, aside(&[("ids", 8)]));
    let read = read_with("Tickets PR-77 and BX-40 closed", &Locale::default(), &rules(true));
    assert!(read.0.is_empty());
    assert_eq!(read.1, aside(&[("ids", 2)]));
}

#[test]
fn numbers_beside_an_identifier_are_still_read() {
    let read = read_with(
        "K7-12.3 rose to 12.5 % from 9.8 %; RQ-3.5 is 3.4 against 3.9",
        &Locale::default(),
        &rules(false),
    );
    assert_eq!(texts(&read), ["12.5 %", "9.8 %", "3.4", "3.9"]);
    assert_eq!(read.1, aside(&[("ids", 2)]));
}

#[test]
fn a_hyphen_between_numbers_is_still_a_range_and_a_word_after_keeps_a_number() {
    let read = read_with(
        "between 2-3.5 and 3.4-3.9, a 12.5-hour shift, a 1.5-fold rise, -2.5 % and v2 with 3 items",
        &Locale::default(),
        &rules(true),
    );
    assert_eq!(texts(&read), ["2", "3.5", "3.4", "3.9", "12.5", "1.5", "-2.5 %", "3"]);
    assert!(read.1.is_empty());
}

#[test]
fn an_identifier_s_digits_are_text_with_a_decimal_comma_too() {
    let read = read_with(
        "Le point K7-12.3 obtient 2,7 sur 5 ; x-1.5 baisse de -2,5 %",
        &nl_be(),
        &rules(false),
    );
    assert_eq!(texts(&read), ["2,7", "-2,5 %"]);
    assert_eq!(read.1, aside(&[("ids", 2), ("whole", 1)]));
}

#[test]
fn a_score_s_scale_is_left_aside_with_whole_numbers_on() {
    let read = read_with("rated 2,7/5 and 3,8 op 5, kost van 12.500", &nl_be(), &rules(true));
    assert_eq!(texts(&read), ["2,7", "3,8", "12.500"]);
    assert_eq!(read.1, aside(&[("scales", 2)]));
}

#[test]
fn a_scale_word_comes_with_its_number() {
    let read = read_with("revenue 2.5m and 1,2 mln", &nl_be(), &rules(false));
    assert_eq!(
        read.0.iter().map(|f| (f.0.as_str(), f.1.as_str())).collect::<Vec<_>>(),
        [("2.5m", "2.5"), ("1,2 mln", "1.2")]
    );
}

#[test]
fn whole_numbers_only_when_the_application_asks() {
    assert_eq!(
        read_with("41 items, 85%", &Locale::default(), &rules(false)).0,
        vec![n("85%", "85", 0, Some(100), false)]
    );
    assert_eq!(
        texts(&read_with("41 items, 85%", &Locale::default(), &rules(true))),
        ["41", "85%"]
    );
}

#[test]
fn the_ignore_list_takes_numbers_and_phrases() {
    let ignore = NumberRules::from_json(&serde_json::json!({"ignore": ["iso 9001.2", "WCAG 2.1", 4.5]})).unwrap();
    let read = read_with("ISO 9001.2 and WCAG 2.1, 4.5 and 4.25", &Locale::default(), &ignore);
    assert_eq!(texts(&read), ["4.25"]);
    assert_eq!(read.1, aside(&[("ignored", 3)]));
}

#[test]
fn written_spans_are_left_out() {
    let text = "Average 2.66 across 3.5 items";
    let (found, _) = find_numbers(text, &Locale::default(), &NumberRules::default(), &[(8, 12)]).unwrap();
    assert_eq!(found.iter().map(|f| f.text.as_str()).collect::<Vec<_>>(), ["3.5"]);
}

#[test]
fn a_number_format_says_its_shown_rounding() {
    assert_eq!(shown(Some("0.0")), Some((1, false, 1)));
    assert_eq!(shown(Some("0.00%")), Some((2, true, 1)));
    assert_eq!(shown(Some("#,##0")), Some((0, false, 1)));
    assert_eq!(shown(Some("0.0,,\"m\"")), Some((1, false, 1_000_000)));
    assert_eq!(shown(Some("[Red]0.0;-0.0")), Some((1, false, 1)));
    assert_eq!(shown(Some("#,##0.00_);(#,##0.00)")), Some((2, false, 1)));
    for code in [
        Some("General"),
        Some("@"),
        Some("0.00E+00"),
        Some("d/m/yyyy"),
        None,
        Some(""),
    ] {
        assert_eq!(shown(code), None, "{code:?}");
    }
}

#[test]
fn the_rules_are_the_application_s() {
    assert!(NumberRules::from_json(&serde_json::json!({"ignore": [[1]]})).is_err());
    assert!(NumberRules::from_json(&serde_json::json!({"texts": 0})).is_err());
    assert!(NumberRules::from_json(&serde_json::json!({"whole": true})).is_err());
    let rules = NumberRules::from_json(&serde_json::json!({"ignore": "27001", "whole_numbers": true})).unwrap();
    let read = read_with("ISO 27001 and 41", &Locale::default(), &rules);
    assert_eq!(texts(&read), ["41"]);
}

#[test]
fn digits_of_another_script_are_read_as_python_reads_them() {
    let read = read_with("rated ٢.٥ here", &Locale::default(), &rules(false));
    assert_eq!(read.0, vec![n("٢.٥", "2.5", 1, None, false)]);
}

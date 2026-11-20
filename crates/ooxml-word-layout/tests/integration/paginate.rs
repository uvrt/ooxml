//! Pagination's rules on made-up documents over one face (support::Mono, 12 pt lines of
//! 2355/2048 em; a page of 2160 twips holds seven): widow and orphan control, a
//! `keepNext` paragraph going with a `keepLines` one, a page break, and an `oddPage`
//! section's blank page and numbers. Every line holds one 17-character word.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::support;

use ooxml_word_layout::paginate::Position;
use support::{engine, model, open, para, run, sect};

fn words(n: usize) -> String {
    vec!["abcdefghijklmnopq"; n].join(" ")
}

fn lay_out(body: &str) -> ooxml_word_layout::DocumentLayout {
    let mut m = model();
    let d = open(&mut m, body, None);
    engine().lay_out(&mut m, &d)
}

fn starts(layout: &ooxml_word_layout::DocumentLayout) -> Vec<Position> {
    layout.pagination.pages.iter().map(|p| p.start.clone()).collect()
}

#[test]
fn a_page_holds_seven_lines() {
    let layout = lay_out(&format!("{}{}", para("", &run(&words(9))), sect("nextPage", 1)));
    assert_eq!(starts(&layout), vec![Position::at(0, 0), Position::at(0, 7)]);
}

#[test]
fn widow_control_takes_two_lines_on() {
    let layout = lay_out(&format!("{}{}", para("", &run(&words(8))), sect("nextPage", 1)));
    assert_eq!(starts(&layout), vec![Position::at(0, 0), Position::at(0, 6)]);
    let off = lay_out(&format!(
        "{}{}",
        para(r#"<w:widowControl w:val="0"/>"#, &run(&words(8))),
        sect("nextPage", 1)
    ));
    assert_eq!(starts(&off), vec![Position::at(0, 0), Position::at(0, 7)]);
}

#[test]
fn keep_next_goes_with_a_kept_paragraph() {
    let body = [
        para("", &run(&words(2))),
        para("<w:keepNext/>", &run(&words(3))),
        para("<w:keepLines/>", &run(&words(5))),
        sect("nextPage", 1),
    ]
    .concat();
    let layout = lay_out(&body);
    // The kept pair moves to the next page; there, taller than the page, it breaks where
    // the page is full.
    assert_eq!(
        starts(&layout),
        vec![Position::at(0, 0), Position::at(1, 0), Position::at(2, 4)]
    );
}

#[test]
fn a_page_break_ends_the_page() {
    let body = [
        para(
            "",
            &format!(r#"{}<w:r><w:br w:type="page"/></w:r>{}"#, run("a"), run("b")),
        ),
        sect("nextPage", 1),
    ]
    .concat();
    let layout = lay_out(&body);
    assert_eq!(starts(&layout), vec![Position::at(0, 0), Position::at(0, 1)]);
    assert!(layout.pagination.pages[0].forced);
}

#[test]
fn an_odd_page_section_after_an_odd_page_gets_a_blank_page() {
    let body = [
        para(&sect("nextPage", 1), &run("one")),
        para("", &run("two")),
        sect("oddPage", 1),
    ]
    .concat();
    let layout = lay_out(&body);
    let info: Vec<(i64, bool)> = layout.pagination.info.iter().map(|i| (i.number, i.blank)).collect();
    assert_eq!(info, vec![(1, false), (2, true), (3, false)]);
}

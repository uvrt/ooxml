//! The facts a relayout takes from the version before ([`LayoutFacts::after`]) when an edit
//! moves every page after it: each paragraph's first page, carried from the version before
//! and found again for the paths the remade pages draw, equals the one a whole layout's
//! facts find -- with the edit on the first page (every page remade, every paragraph moved
//! to another page) and in the middle (the pages before it kept, their paragraphs' first
//! pages carried as they were).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::support::{engine, model, open, para, run, sect};
use ooxml_word_layout::facts::LayoutFacts;

/// `n` words.
fn words(n: usize) -> String {
    vec!["word"; n].join(" ")
}

/// Paragraphs of 3 to 9 lines on the small page, the one at `grown` `extra` words longer.
fn body(grown: usize, extra: usize) -> String {
    let mut xml = String::new();
    for k in 0..90 {
        let n = 12 + (k * 7) % 30 + if k == grown { extra } else { 0 };
        xml.push_str(&para("", &run(&words(n))));
    }
    xml.push_str(&sect("nextPage", 1));
    xml
}

/// The first pages `LayoutFacts::after` carries for `after`'s body laid out again from
/// `before`'s equal those of its whole layout; how many paragraphs' first pages moved.
fn carried_equal_whole(before: &str, after: &str) -> usize {
    let mut kept_model = model();
    let mut kept = engine();
    let first = open(&mut kept_model, before, None);
    let previous = kept.lay_out(&mut kept_model, &first);
    let known = LayoutFacts::of(&previous);
    let old = known.first_pages().clone();
    let document = open(&mut kept_model, after, None);
    let relaid = kept.relayout(&mut kept_model, &document, &previous);
    let carried = LayoutFacts::after(&relaid, &previous, &known);
    let mut fresh_model = model();
    let fresh_document = open(&mut fresh_model, after, None);
    let whole = engine().lay_out(&mut fresh_model, &fresh_document);
    let made = LayoutFacts::of(&whole);
    assert!(carried == made, "the facts taken from the version before differ");
    assert_eq!(carried.first_pages(), made.first_pages(), "the first pages differ");
    made.first_pages()
        .iter()
        .filter(|(path, page)| old.get(*path) != Some(page))
        .count()
}

#[test]
fn first_pages_carried_when_an_edit_on_the_first_page_moves_every_page() {
    let moved = carried_equal_whole(&body(0, 0), &body(0, 25));
    assert!(moved > 60, "most paragraphs start on another page: {moved}");
}

#[test]
fn first_pages_carried_when_an_edit_in_the_middle_moves_the_pages_after_it() {
    let moved = carried_equal_whole(&body(45, 0), &body(45, 25));
    assert!(
        moved > 20 && moved < 60,
        "the paragraphs after the edit start on another page: {moved}"
    );
}

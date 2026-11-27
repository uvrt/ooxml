//! A strict Open XML document laid out (phase 3 RD1a): Word's Save As Strict of
//! `constructs.docx` beside its Save As Word Document (`make_strict_probe`,
//! `crates/ooxml-oracle/word/observations/strict-word.json`). The layout reads a strict
//! document through the namespace-exact views, whose namespaces hold strict's URIs, and the
//! universal measures strict writes (`35.40pt`) as Word reads them:
//!
//! - `a_strict_document_lays_out_as_its_transitional_twin`: every glyph the layout draws of
//!   the strict twin, where it draws it on the transitional one, as far as the views carry
//!   the reading ([`READ_GAPS`]: strict forms the readers not yet ported onto the views
//!   would read otherwise, which this document does not hold);
//! - `the_strict_twin_agrees_with_word_as_the_transitional_one_does` (end gate): the
//!   layout's lines held to Word's of each twin (as `baselines` holds them), the strict one
//!   agreeing as far as the transitional one does.

use super::glyphs::{laid_glyphs, Glyph};
use super::lines::text_lines;
use super::{lay_out, probe, Scored};
use serde_json::Value;

/// What the readers not ported onto the views yet (`read/`, the props reader: RD2 and PJ)
/// may read of a strict document otherwise than of its transitional twin -- strict forms
/// this probe's document does not hold, so this test cannot see them: each the values and
/// why.
pub const READ_GAPS: &[(&str, &str)] = &[
    (
        "half-point sizes (`w:sz`, `w:kern`, `w:position`)",
        "strict allows a universal measure (`11pt`); the run reader reads an integer (Word writes integers)",
    ),
    (
        "DrawingML coordinates (`a:off`, `a:ext`, `wp:posOffset`)",
        "strict allows a universal measure for an EMU coordinate; the drawing readers read integers",
    ),
    (
        "percentages other than widths (`w:shd` patterns, `w:w` text scale)",
        "strict writes `50%` where transitional writes fiftieths or a number",
    ),
];

fn glyphs(name: &str) -> Vec<Glyph> {
    laid_glyphs(&lay_out(name, probe("make_strict_probe", name.trim_end_matches(".docx"))).pages)
}

/// A glyph as the two layouts are held to each other: its page, character, pen and
/// baseline to 1/100 px, face and size.
fn drawn(glyph: &Glyph) -> (usize, char, i64, i64, String, i64) {
    let hundredths = |v: f64| (v * 100.0).round() as i64;
    (
        glyph.page,
        glyph.ch,
        hundredths(glyph.x),
        hundredths(glyph.y),
        glyph.face.clone(),
        hundredths(glyph.size_px),
    )
}

#[test]
fn a_strict_document_lays_out_as_its_transitional_twin() {
    let strict: Vec<_> = glyphs("strict.docx").iter().map(drawn).collect();
    let transitional: Vec<_> = glyphs("transitional.docx").iter().map(drawn).collect();
    assert!(!strict.is_empty(), "the strict twin lays out nothing");
    let first = strict.iter().zip(&transitional).position(|(a, b)| a != b);
    assert!(
        first.is_none() && strict.len() == transitional.len(),
        "{} glyphs against {}; first apart at {first:?}: {:?} against {:?} (a strict form read otherwise? {READ_GAPS:?})",
        strict.len(),
        transitional.len(),
        first.and_then(|k| strict.get(k)),
        first.and_then(|k| transitional.get(k)),
    );
}

#[test]
#[ignore = "end gate: the layout against Word's recording of the twins"]
fn the_strict_twin_agrees_with_word_as_the_transitional_one_does() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/word/observations/strict-word.json");
    let recording: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let score = |name: &str| {
        let mut scored = Scored::default();
        let laid = lay_out(name, probe("make_strict_probe", name.trim_end_matches(".docx")));
        text_lines(&mut scored, "lines", &recording["documents"][name], &laid.pages);
        scored.cases["lines"]
    };
    let (strict, transitional) = (score("strict.docx"), score("transitional.docx"));
    assert!(strict.1 > 0, "Word drew no line of the strict twin");
    assert_eq!(strict, transitional, "(lines the layout misses, Word's lines)");
}

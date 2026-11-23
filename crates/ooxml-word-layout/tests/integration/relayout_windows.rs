//! Relayout equals full layout where the flows differ in several windows of other lengths
//! (`incremental::repaginate` joining a page past several changes): a body of three alike
//! copies of the same paragraphs, aligned by content.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::relayout::verify;
use crate::support::{engine, model, open, para, run};

/// A body of three alike copies of the same paragraphs: one paragraph of the second copy
/// made longer and one of the third deleted. The bodies align by content, so the changes
/// between the flows are several windows -- one adding as many items as a copy holds (the
/// old paragraph is found again in the third copy), a later one taking them away again --
/// and a new page past a later window lines up with an old one by that window's shift, not
/// the first's (it kept stale pages at the end).
#[test]
fn a_page_past_several_windows_joins_by_the_last_window_s_shift() {
    let words = |k: usize| "word ".repeat(3 + (k * 7) % 40);
    let copy: Vec<String> = (0..24).map(|k| para("", &run(&words(k)))).collect();
    let three: Vec<String> = copy.iter().chain(&copy).chain(&copy).cloned().collect();
    for longer in [9, 15, 21] {
        let mut m = model();
        let mut e = engine();
        let d = open(&mut m, &three.concat(), None);
        let before = e.lay_out(&mut m, &d);
        let mut blocks = three.clone();
        blocks[24 + longer] = para("", &run(&format!("{} and a few more words", words(longer))));
        blocks.remove(48 + 14);
        let body = blocks.concat();
        let d = open(&mut m, &body, None);
        let after = e.relayout(&mut m, &d, &before);
        let at = format!("paragraph {longer} longer");
        verify(&at, (&mut e, &mut m), (&after, &before), (&body, None, &[]));
    }
}

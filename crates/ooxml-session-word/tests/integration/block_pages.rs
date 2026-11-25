//! `Session::block_pages` (lane E2): the pages a Word document's head version draws body
//! blocks on, laid out through the call's layout -- what a host measures once its batch has
//! run (docx-agent's `word_format_table` keep_together, said after the last op); a block the
//! version does not draw is said not known, and without a layout (the host's or the
//! session's own) nothing is laid out.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::layout;
use crate::locale::{docx, session};
use ooxml_session::{CallContext, DocKind};
use ooxml_session_word::prelude::*;
use ooxml_session_word::BlockAt;
use ooxml_session_word::WordHost;

#[test]
fn a_head_version_s_blocks_are_said_on_their_pages() {
    let s = session(None);
    let doc = s.open(docx(None), DocKind::Docx, "d.docx").unwrap();
    let context = CallContext {
        host: WordHost {
            layout: Some(layout()),
            ..WordHost::default()
        }
        .into_host(),
        ..CallContext::default()
    };
    let blocks = ["p:10000001".to_owned(), "p:7FFFFFFF".to_owned()];
    let found = s.block_pages(&doc, &blocks, &context).unwrap().expect("laid out");
    assert_eq!(found[0], BlockAt::Pages(vec![1])); // numbered from 1
    assert!(matches!(found[1], BlockAt::Unknown(_)), "{found:?}");
    // without the host's layout or a session engine, nothing is laid out
    let unlaid = s.block_pages(&doc, &blocks, &CallContext::default()).unwrap();
    assert_eq!(unlaid, Err("no layout".to_owned()));
}

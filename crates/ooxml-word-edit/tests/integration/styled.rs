//! `word_set_text`'s styled text (tool design rule 3): writing a paragraph's text with
//! inline emphasis twice changes nothing the second time -- the text is there and its
//! pieces look as asked already; and the styles are added once, as Word writes them.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Monotonic, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_word_edit::{Editor, RevisionPolicy, SetStyledText};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[test]
fn a_repeat_writes_nothing() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python/docx-agent/tests/fixtures/samplelib/sample-long.docx");
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let package = Package::open(bytes, &Limits::default()).unwrap();
    let editor = Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(Monotonic(Instant::now())),
        Arc::new(Unlimited),
    )
    .unwrap();
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let target = docx.body().unwrap().ids.paragraphs().iter().next().unwrap().id.clone();
    let command = SetStyledText {
        target,
        text: "Plain **strong** and *emphasis* [named]{Book Title}".to_owned(),
        paragraph_style: None,
        policy: RevisionPolicy::Untracked,
    };
    let first = editor.run(&Caller::anonymous(), &command).unwrap();
    assert!(first.out.edits.iter().any(|e| e.changed));
    assert_eq!(first.out.styled.len(), 3);
    let command = SetStyledText {
        target: first.out.written[0].clone(),
        ..command
    };
    let again = editor.run(&Caller::anonymous(), &command).unwrap();
    assert!(again.out.edits.iter().all(|e| !e.changed), "{:?}", again.out.edits);
    assert!(again.out.notes.is_empty());
}

//! What a list edit depends on (`lists::depends`): a paragraph's text set between its
//! snapshot and its commit is no conflict where the paragraph is in no list and not one it
//! names -- it commits on its first preparation, rebased -- and is one where the paragraph is
//! in a list; either way the document equals the two edits run one after the other.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{fixture, written, Monotonic, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, Command, StoreConfig, Txn};
use ooxml_word_edit::{
    numbered_by_style, EditError, Editor, ListAction, ListEdit, ListsEdited, RevisionPolicy, SetText, WordDoc,
};
use ooxml_word_model::docx::find_path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

fn editor() -> Editor {
    let bytes = fixture("generated/lists-and-styles.docx");
    Editor::open(
        Package::open(bytes, &Limits::default()).unwrap(),
        StoreConfig::default(),
        std::sync::Arc::new(Monotonic(Instant::now())),
        std::sync::Arc::new(Unlimited),
    )
    .unwrap()
}

/// The body's top-level paragraphs: each one's id and whether it is in a list.
fn paragraphs(editor: &Editor) -> Vec<(String, bool)> {
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let body = docx.body().unwrap();
    body.ids
        .paragraphs()
        .iter()
        .filter(|p| p.path.len() == 2)
        .map(|p| {
            let listed =
                find_path(&p.element, &["pPr", "numPr"]).is_some() || numbered_by_style(&docx, &p.element).is_some();
            (p.id.clone(), listed)
        })
        .collect()
}

/// `list`, with `meanwhile` committed by another writer after its first preparation began.
struct Meanwhile<'e> {
    editor: &'e Editor,
    list: ListEdit,
    meanwhile: SetText,
    done: AtomicBool,
}

impl Command<WordDoc> for Meanwhile<'_> {
    type Out = ListsEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        self.list.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<ListsEdited, EditError> {
        let out = self.list.prepare(tx)?;
        if !self.done.swap(true, Ordering::SeqCst) {
            self.editor.run(&Caller::named("other"), &self.meanwhile).unwrap();
        }
        Ok(out)
    }
}

/// The list edit's attempts, and whether the document equals the set text, then the list
/// edit, run alone.
fn raced(target: &str, text_of: &str) -> (u32, bool) {
    let list = || ListEdit {
        targets: vec![target.to_owned()],
        action: ListAction::Add {
            kind: "bullet".to_owned(),
            level: 0,
        },
        policy: RevisionPolicy::Untracked,
    };
    let set = || SetText {
        target: text_of.to_owned(),
        text: "Set meanwhile.".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    let shared = editor();
    let command = Meanwhile {
        editor: &shared,
        list: list(),
        meanwhile: set(),
        done: AtomicBool::new(false),
    };
    let done = shared.run(&Caller::named("lists"), &command).unwrap();
    let serial = editor();
    serial.run(&Caller::anonymous(), &set()).unwrap();
    serial.run(&Caller::anonymous(), &list()).unwrap();
    let same = written(shared.store().snapshot().doc()) == written(serial.store().snapshot().doc());
    (done.attempts, same)
}

#[test]
fn a_text_set_outside_every_list_meanwhile_is_no_conflict() {
    let found = paragraphs(&editor());
    let plain: Vec<&String> = found.iter().filter(|(_, listed)| !listed).map(|(id, _)| id).collect();
    assert!(plain.len() >= 2, "{found:?}");
    let (attempts, same) = raced(plain[0], plain[plain.len() - 1]);
    assert_eq!(attempts, 1, "prepared again for an edit it does not depend on");
    assert!(same, "not the serial order's document");
}

#[test]
fn a_text_set_in_a_list_meanwhile_is_a_conflict() {
    let found = paragraphs(&editor());
    let plain = found.iter().find(|(_, listed)| !listed).map(|(id, _)| id).unwrap();
    let listed = found.iter().find(|(_, listed)| *listed).map(|(id, _)| id).unwrap();
    let (attempts, same) = raced(plain, listed);
    assert_eq!(attempts, 2, "committed past an edit of a list it read");
    assert!(same, "not the serial order's document");
}

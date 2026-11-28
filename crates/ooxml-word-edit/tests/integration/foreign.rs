//! Another editor's version of a document coming in (co-authoring refresh,
//! `Editor::receive`): its blocks arrive with the ids that editor wrote and the blocks it did
//! not touch keep theirs; with nothing changed here since its base the document saves as
//! that editor's file, byte for byte (nothing travels back); an edit prepared before it on a
//! block it changed is refused, naming the block, and one on another block is rebased past
//! it; the steps since its base that touched its blocks are handed back; an id it wrote that
//! a draw here gave meanwhile is stamped anew where it lands.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use crate::interleave::Paused;
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, Seq, Snapshot, StoreError};
use ooxml_word_edit::{EditError, Editor, InsertParagraph, Key, RevisionPolicy, SetText, WordDoc};
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const MAIN: &str = "word/document.xml";

fn set_text(target: &str, text: &str) -> SetText {
    SetText {
        target: ooxml_word_model::address::Address::from(target.to_owned()),
        text: text.to_owned(),
        policy: RevisionPolicy::Untracked,
    }
}

/// The body's top-level paragraphs, by id.
fn paragraphs(doc: &WordDoc) -> Vec<String> {
    let docx = doc.docx().unwrap();
    let body = docx.body().unwrap();
    body.ids
        .paragraphs()
        .iter()
        .filter(|p| p.path.len() == 2)
        .map(|p| p.id.clone())
        .collect()
}

/// The paragraph `id`'s element in `doc`.
fn paragraph(doc: &WordDoc, id: &str) -> Arc<ooxml_xml::Element> {
    let docx = doc.docx().unwrap();
    let body = docx.body().unwrap();
    let found = body.ids.paragraphs().iter().find(|p| p.id == id).unwrap();
    Arc::clone(&found.element)
}

fn text_of(element: &ooxml_xml::Element) -> String {
    element.text_content()
}

/// The other editor's file: `base` saved, opened by another editor, `edit` run there, saved.
fn theirs(base: &Snapshot<WordDoc>, edit: impl Fn(&Editor)) -> Arc<[u8]> {
    let bytes: Arc<[u8]> = base.doc().package().save().unwrap().into();
    let other = editor(&bytes);
    edit(&other);
    other.store().snapshot().doc().package().save().unwrap().into()
}

fn open(bytes: &Arc<[u8]>) -> Package {
    Package::open(Arc::clone(bytes), &Limits::default()).unwrap()
}

/// An editor of a fixture after a first edit (every block stamped and written), its
/// top-level paragraphs.
fn opened() -> (Editor, Vec<String>) {
    let ours = editor(&fixture("generated/ids-and-markup.docx"));
    let first = paragraphs(ours.store().snapshot().doc()).last().cloned().unwrap();
    ours.run(&Caller::anonymous(), &set_text(&first, "Written first."))
        .unwrap();
    let targets = paragraphs(ours.store().snapshot().doc());
    assert!(targets.len() >= 4, "{targets:?}");
    (ours, targets)
}

#[test]
fn their_blocks_arrive_with_their_ids_and_the_others_keep_theirs() {
    let (ours, targets) = opened();
    let base = ours.store().snapshot();
    let bytes = theirs(&base, |other| {
        other
            .run(&Caller::anonymous(), &set_text(&targets[0], "Their words."))
            .unwrap();
    });
    let arrived = ours.receive("Ada", base.seq(), open(&bytes)).unwrap();
    assert!(arrived.handed_back.is_empty());
    assert!(arrived.blocks.contains(&targets[0]), "{:?}", arrived.blocks);
    assert!(arrived.parts.iter().any(|p| p == MAIN));
    let head = ours.store().snapshot();
    let mine = paragraph(head.doc(), &targets[0]);
    let their = paragraph(editor(&bytes).store().snapshot().doc(), &targets[0]);
    assert!(text_of(&mine).contains("Their words."));
    assert_eq!(
        mine.attribute(Some(W14), "textId"),
        their.attribute(Some(W14), "textId")
    );
    assert_eq!(
        mine.attribute(Some(W14), "paraId"),
        their.attribute(Some(W14), "paraId")
    );
    // a block they did not touch is the very element it was
    assert!(Arc::ptr_eq(
        &paragraph(head.doc(), &targets[1]),
        &paragraph(base.doc(), &targets[1])
    ));
    let step = ours.store().steps().last().cloned().unwrap();
    assert!(step.foreign);
    assert_eq!(step.caller.as_deref(), Some("Ada"));
}

#[test]
fn with_nothing_changed_here_the_document_saves_as_their_file() {
    let (ours, targets) = opened();
    let base = ours.store().snapshot();
    let bytes = theirs(&base, |other| {
        other
            .run(&Caller::anonymous(), &set_text(&targets[2], "Echo."))
            .unwrap();
    });
    ours.receive("Ada", base.seq(), open(&bytes)).unwrap();
    let saved = ours.store().snapshot().doc().package().save().unwrap();
    assert!(*saved == *bytes, "the refreshed document saves as their file");
}

/// `mine` prepared and held, their change of `targets[0]` received meanwhile, then `mine`
/// committed: its result.
fn across(ours: &Editor, base: Seq, bytes: &Arc<[u8]>, mine: SetText) -> Result<u32, EditError> {
    let (prepared, wait) = channel();
    let (release, go) = channel();
    let paused = Paused {
        command: mine,
        prepared,
        go: Mutex::new(go),
        first: AtomicBool::new(true),
    };
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| ours.run(&Caller::named("agent"), &paused).map(|done| done.attempts));
        wait.recv().unwrap();
        ours.receive("Ada", base, open(bytes)).unwrap();
        release.send(()).unwrap();
        handle.join().unwrap()
    })
}

#[test]
fn an_edit_prepared_before_their_change_of_its_block_is_refused_naming_it() {
    let (ours, targets) = opened();
    let base = ours.store().snapshot();
    let bytes = theirs(&base, |other| {
        other
            .run(&Caller::anonymous(), &set_text(&targets[0], "Theirs."))
            .unwrap();
    });
    match across(&ours, base.seq(), &bytes, set_text(&targets[0], "Mine.")) {
        Err(EditError::Store(StoreError::ForeignChange { editor, overlap, .. })) => {
            assert_eq!(&*editor, "Ada");
            assert!(overlap.contains(&Key::block(MAIN, targets[0].as_str())), "{overlap:?}");
        }
        other => panic!("not refused: {other:?}"),
    }
    assert!(text_of(&paragraph(ours.store().snapshot().doc(), &targets[0])).contains("Theirs."));
}

#[test]
fn an_edit_prepared_before_their_change_of_another_block_is_rebased_past_it() {
    let (ours, targets) = opened();
    let base = ours.store().snapshot();
    let bytes = theirs(&base, |other| {
        other
            .run(&Caller::anonymous(), &set_text(&targets[0], "Theirs."))
            .unwrap();
    });
    let attempts = across(&ours, base.seq(), &bytes, set_text(&targets[1], "Mine.")).unwrap();
    assert_eq!(attempts, 1, "rebased, not prepared again");
    let head = ours.store().snapshot();
    assert!(text_of(&paragraph(head.doc(), &targets[0])).contains("Theirs."));
    assert!(text_of(&paragraph(head.doc(), &targets[1])).contains("Mine."));
}

#[test]
fn steps_here_that_touched_their_blocks_are_handed_back_and_the_others_stay() {
    let (ours, targets) = opened();
    let base = ours.store().snapshot();
    let touched = ours
        .run(&Caller::named("agent"), &set_text(&targets[0], "Mine."))
        .unwrap();
    ours.run(&Caller::named("other agent"), &set_text(&targets[1], "Kept."))
        .unwrap();
    let bytes = theirs(&base, |other| {
        other
            .run(&Caller::anonymous(), &set_text(&targets[0], "Theirs."))
            .unwrap();
    });
    let arrived = ours.receive("Ada", base.seq(), open(&bytes)).unwrap();
    assert_eq!(arrived.handed_back.len(), 1);
    let back = &arrived.handed_back[0];
    assert_eq!(back.step.step, touched.version.seq());
    assert_eq!(back.step.caller.as_deref(), Some("agent"));
    let head = ours.store().snapshot();
    assert!(text_of(&paragraph(head.doc(), &targets[0])).contains("Theirs."));
    assert!(text_of(&paragraph(head.doc(), &targets[1])).contains("Kept."));
    // brought back by hand it would overwrite theirs: refused, naming them
    assert!(matches!(
        ours.store().redo_step(back.step.step),
        Err(StoreError::Entangled { .. } | StoreError::ForeignChange { .. })
    ));
}

#[test]
fn an_id_they_wrote_that_a_draw_here_gave_meanwhile_is_stamped_anew() {
    let (ours, targets) = opened();
    let base = ours.store().snapshot();
    let before: HashSet<String> = paragraphs(base.doc()).into_iter().collect();
    let insert = |anchor: &str| InsertParagraph {
        anchor: anchor.to_owned(),
        after: true,
        text: "New here.".to_owned(),
        style: None,
        policy: RevisionPolicy::Untracked,
    };
    let mine = ours.run(&Caller::named("agent"), &insert(&targets[1])).unwrap();
    let drawn = mine.out.id.to_string();
    // theirs: a paragraph of their own, written with the paraId drawn here
    let bytes = theirs(&base, |other| {
        other.run(&Caller::anonymous(), &insert(&targets[3])).unwrap();
    });
    let their_doc = editor(&bytes);
    let new: Vec<String> = paragraphs(their_doc.store().snapshot().doc())
        .into_iter()
        .filter(|p| !before.contains(p))
        .collect();
    let (their_id, our_id) = (new[0].trim_start_matches("p:"), drawn.trim_start_matches("p:"));
    let package = open(&bytes);
    let xml = String::from_utf8(package.read(MAIN).unwrap().unwrap().to_vec()).unwrap();
    let same_id = xml.replace(their_id, our_id);
    let package = package.with_bytes(MAIN, same_id.into_bytes().into());
    let bytes: Arc<[u8]> = package.save().unwrap().into();
    ours.receive("Ada", base.seq(), open(&bytes)).unwrap();
    let head = ours.store().snapshot();
    let docx = head.doc().docx().unwrap();
    assert!(
        docx.parts.iter().all(|p| p.ids.fully_stamped()),
        "every block keeps a unique id"
    );
    let texts = text_of(&docx.body().unwrap().tree.root().clone());
    assert_eq!(texts.matches("New here.").count(), 2, "both paragraphs are there");
    assert!(paragraphs(head.doc()).contains(&drawn), "the block here keeps its id");
}

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
use ooxml_word_edit::{
    EditError, Editor, InsertParagraph, InsertPicture, Key, RevisionPolicy, SetText, Tracking, WordDoc,
};
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
    // their block, stamped anew, is not written under the id ours holds
    let step = ours.store().steps().last().cloned().unwrap();
    assert!(
        !step.writes.contains(&Key::block(MAIN, drawn.as_str())),
        "{:?}",
        step.writes
    );
}

#[test]
fn their_file_the_same_as_its_base_brings_nothing_and_hands_nothing_back() {
    let (ours, targets) = opened();
    let base = ours.store().snapshot();
    let bytes: Arc<[u8]> = base.doc().package().save().unwrap().into();
    // a step here that read the whole document (every match of a find)
    let replace = ooxml_word_edit::ReplaceText {
        pattern: Arc::new(ooxml_word_edit::find::Literal::new("Written", true, false, "omega").unwrap()),
        across: false,
        within: None,
        count: None,
        keep: ooxml_word_edit::Keep::First,
        policy: RevisionPolicy::Untracked,
    };
    let _ = targets;
    let done = ours.run(&Caller::named("agent"), &replace).unwrap();
    assert!(done.changed);
    let arrived = ours.receive("Ada", base.seq(), open(&bytes)).unwrap();
    assert!(arrived.previous.is_none() && arrived.handed_back.is_empty() && arrived.parts.is_empty());
    assert_eq!(ours.store().snapshot().seq(), done.version.seq());
}

fn tracked(target: &str, text: &str, author: &str) -> SetText {
    SetText {
        target: ooxml_word_model::address::Address::from(target.to_owned()),
        text: text.to_owned(),
        policy: RevisionPolicy::Tracked(Tracking {
            author: author.to_owned(),
            date: "2026-11-28T10:00:00Z".to_owned(),
        }),
    }
}

/// The authors the people part lists.
fn people(doc: &WordDoc) -> Vec<String> {
    let package = doc.package();
    let Some(name) = package.part_names().into_iter().find(|n| n.contains("people")) else {
        return Vec::new();
    };
    let tree = package.tree(&name).unwrap().unwrap();
    let w15 = "http://schemas.microsoft.com/office/word/2012/wordml";
    let mut out: Vec<String> = tree
        .root()
        .child_elements()
        .filter_map(|e| e.attribute(Some(w15), "author").map(str::to_owned))
        .collect();
    out.sort();
    out
}

#[test]
fn their_change_of_the_first_tracked_edits_block_hands_back_that_edit_alone() {
    let (ours, targets) = opened();
    let base = ours.store().snapshot();
    // the first tracked edit declares the root's namespaces and lists its author; the second
    // lists another: neither ties the other (both only merges there)
    let first = ours
        .run(&Caller::named("a"), &tracked(&targets[0], "Mine, tracked.", "Ann"))
        .unwrap();
    ours.run(&Caller::named("b"), &tracked(&targets[1], "Bo's, tracked.", "Bo"))
        .unwrap();
    let bytes = theirs(&base, |other| {
        other
            .run(&Caller::anonymous(), &set_text(&targets[0], "Theirs."))
            .unwrap();
    });
    let arrived = ours.receive("Ada", base.seq(), open(&bytes)).unwrap();
    let back: Vec<Seq> = arrived.handed_back.iter().map(|h| h.step.step).collect();
    assert_eq!(back, vec![first.version.seq()], "only the edit of their block");
    let head = ours.store().snapshot();
    assert!(
        text_of(&paragraph(head.doc(), &targets[1])).contains("tracked"),
        "Bo's edit stays"
    );
    // (Ann stays listed: a person's tidy lands before the undo's text there, where Ann's
    // revisions are still in use; a person listed and unused is harmless)
    assert!(people(head.doc()).contains(&"Bo".to_owned()));
}

#[test]
fn their_root_declaring_more_is_merged_and_the_part_stays_patched() {
    let (ours, targets) = opened();
    let base = ours.store().snapshot();
    ours.run(&Caller::named("a"), &set_text(&targets[2], "Kept here."))
        .unwrap();
    // theirs: one paragraph set, and their root declaring a namespace more (ignorable)
    let edited = theirs(&base, |other| {
        other
            .run(&Caller::anonymous(), &set_text(&targets[0], "Theirs."))
            .unwrap();
    });
    let package = open(&edited);
    let xml = String::from_utf8(package.read(MAIN).unwrap().unwrap().to_vec()).unwrap();
    let declared = xml.replacen("<w:document ", "<w:document xmlns:w16x=\"urn:example:w16x\" ", 1);
    let declared = match declared.find("mc:Ignorable=\"") {
        Some(at) => {
            let at = at + "mc:Ignorable=\"".len();
            format!("{}w16x {}", &declared[..at], &declared[at..])
        }
        None => declared,
    };
    let bytes: Arc<[u8]> = package
        .with_bytes(MAIN, declared.into_bytes().into())
        .save()
        .unwrap()
        .into();
    let arrived = ours.receive("Ada", base.seq(), open(&bytes)).unwrap();
    assert!(arrived.handed_back.is_empty(), "{:?}", arrived.handed_back.len());
    let head = ours.store().snapshot();
    let tree = head.doc().package().tree(MAIN).unwrap().unwrap();
    assert!(
        tree.root().namespaces().iter().any(|d| &*d.uri == "urn:example:w16x"),
        "their declaration merged"
    );
    assert!(text_of(&paragraph(head.doc(), &targets[2])).contains("Kept here."));
    assert!(text_of(&paragraph(head.doc(), &targets[0])).contains("Theirs."));
}

#[test]
fn people_both_sides_list_come_together_by_author() {
    let (ours, targets) = opened();
    // a person listed before the base
    ours.run(&Caller::named("a"), &tracked(&targets[3], "Before.", "Ann"))
        .unwrap();
    let base = ours.store().snapshot();
    ours.run(&Caller::named("b"), &tracked(&targets[1], "Bo's.", "Bo"))
        .unwrap();
    let bytes = theirs(&base, |other| {
        other
            .run(&Caller::anonymous(), &tracked(&targets[0], "Ada's.", "Ada"))
            .unwrap();
    });
    let arrived = ours.receive("Ada", base.seq(), open(&bytes)).unwrap();
    assert!(arrived.handed_back.is_empty());
    assert_eq!(people(ours.store().snapshot().doc()), ["Ada", "Ann", "Bo"]);
}

/// A one-pixel PNG of the colour `rgb`.
fn png(rgb: [u8; 3]) -> Arc<[u8]> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    }
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn adler(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for byte in data {
            a = (a + u32::from(*byte)) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }
    let raw = [0u8, rgb[0], rgb[1], rgb[2]];
    // a stored deflate block of the one scanline
    let mut idat = vec![0x78, 0x01, 0x01, 4, 0, 0xFB, 0xFF];
    idat.extend_from_slice(&raw);
    idat.extend_from_slice(&adler(&raw).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out.into()
}

fn picture(target: &str, image: Arc<[u8]>) -> InsertPicture {
    InsertPicture {
        at: ooxml_word_model::address::Address::from(format!("{target}@0")),
        image,
        width: Some(20.0),
        alt_text: None,
        policy: RevisionPolicy::Untracked,
    }
}

/// The pictures the main part draws: each one's relationship id and the bytes it targets.
fn pictures(doc: &WordDoc) -> Vec<(String, Arc<[u8]>)> {
    let package = doc.package();
    let tree = package.tree(MAIN).unwrap().unwrap();
    let r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let a = "http://schemas.openxmlformats.org/drawingml/2006/main";
    let rels = package.relationships(MAIN).unwrap();
    let mut out = Vec::new();
    let mut stack = vec![Arc::clone(tree.root())];
    while let Some(e) = stack.pop() {
        if e.name().is(Some(a), "blip") {
            if let Some(id) = e.attribute(Some(r), "embed") {
                let target = rels
                    .iter()
                    .find(|rel| rel.id == id)
                    .and_then(|rel| rel.target_part.clone())
                    .unwrap();
                out.push((id.to_owned(), package.read(&target).unwrap().unwrap()));
            }
        }
        stack.extend(e.child_elements().cloned());
    }
    out
}

#[test]
fn two_editors_each_adding_a_picture_after_one_base_keep_both() {
    let (ours, targets) = opened();
    let base = ours.store().snapshot();
    let (mine, theirs_image) = (png([200, 10, 10]), png([10, 10, 200]));
    ours.run(&Caller::named("a"), &picture(&targets[1], Arc::clone(&mine)))
        .unwrap();
    let bytes = theirs(&base, |other| {
        other
            .run(&Caller::anonymous(), &picture(&targets[3], Arc::clone(&theirs_image)))
            .unwrap();
    });
    // both picked the same next id and the same media name
    let their_pictures = pictures(editor(&bytes).store().snapshot().doc());
    let our_pictures = pictures(ours.store().snapshot().doc());
    assert_eq!(their_pictures[0].0, our_pictures[0].0, "the same rId on both sides");
    let arrived = ours.receive("Ada", base.seq(), open(&bytes)).unwrap();
    assert!(arrived.handed_back.is_empty(), "nothing handed back: ours renamed");
    let head = ours.store().snapshot();
    let drawn = pictures(head.doc());
    assert_eq!(
        drawn.len(),
        2,
        "{:?}",
        drawn.iter().map(|(id, _)| id).collect::<Vec<_>>()
    );
    assert_ne!(drawn[0].0, drawn[1].0, "each its own id");
    let images: Vec<&Arc<[u8]>> = drawn.iter().map(|(_, b)| b).collect();
    assert!(
        images.contains(&&mine) && images.contains(&&theirs_image),
        "each draws its own picture"
    );
    // the document saves and opens again with both
    let saved: Arc<[u8]> = head.doc().package().save().unwrap().into();
    assert_eq!(pictures(editor(&saved).store().snapshot().doc()).len(), 2);
}

#[test]
fn a_picture_prepared_before_theirs_comes_in_is_prepared_again_not_refused() {
    let (ours, targets) = opened();
    let base = ours.store().snapshot();
    let (mine, theirs_image) = (png([10, 200, 10]), png([200, 200, 10]));
    let bytes = theirs(&base, |other| {
        other
            .run(&Caller::anonymous(), &picture(&targets[3], Arc::clone(&theirs_image)))
            .unwrap();
    });
    let (prepared, wait) = channel();
    let (release, go) = channel();
    let paused = Paused {
        command: picture(&targets[1], Arc::clone(&mine)),
        prepared,
        go: Mutex::new(go),
        first: AtomicBool::new(true),
    };
    // the relationships and the media name it picked are bookkeeping theirs merged: it is
    // prepared again where theirs landed, not refused as changed meanwhile
    let attempts = std::thread::scope(|scope| {
        let handle = scope.spawn(|| ours.run(&Caller::named("agent"), &paused).map(|done| done.attempts));
        wait.recv().unwrap();
        ours.receive("Ada", base.seq(), open(&bytes)).unwrap();
        release.send(()).unwrap();
        handle.join().unwrap()
    })
    .unwrap();
    assert!(attempts >= 1);
    let drawn = pictures(ours.store().snapshot().doc());
    assert_eq!(drawn.len(), 2);
    assert_ne!(drawn[0].0, drawn[1].0);
}

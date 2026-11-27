//! A document is stamped in memory when it is opened (stamp-on-open design, 2026-11-26,
//! section 7's properties): its ids are the bytes' own (opened twice, the same); a save
//! before any edit is the bytes; the first edit writes every story part and renames
//! nothing; an edit saved and opened again keeps every id; an undo back to the version
//! opened saves the bytes again; and two first edits made at once both commit (writing the
//! ids given at open is no conflict).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::{Editor, RevisionPolicy, SetText, Setup, WordDoc};
use std::sync::Arc;

/// Unstamped fixtures: a row without a paraId, paragraphs without, a drawing without ids.
const UNSTAMPED: [&str; 3] = [
    "generated/toc-numbered.docx",
    "samplelib/sample-long.docx",
    "samplelib/sample-resume.docx",
];

/// Every story block's id, part by part.
fn ids(doc: &WordDoc) -> Vec<(String, Vec<String>)> {
    let docx = doc.docx().unwrap();
    docx.parts
        .iter()
        .map(|p| {
            let mut ids: Vec<String> = p.ids.paragraphs().iter().map(|q| q.id).collect();
            for table in p.ids.tables().iter() {
                ids.push(table.id.clone());
                ids.extend(table.rows.iter().map(|(_, r)| r.clone()));
            }
            (p.part.clone(), ids)
        })
        .collect()
}

/// The body's top-level paragraphs' ids.
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

fn set_text(target: &str, text: &str) -> SetText {
    SetText {
        target: ooxml_word_model::address::Address::from(target.to_owned()),
        text: text.to_owned(),
        policy: RevisionPolicy::Untracked,
    }
}

fn reopened(bytes: Vec<u8>) -> Editor {
    editor(&Arc::from(bytes))
}

#[test]
fn an_unstamped_document_reads_durable_ids_the_same_each_time_and_saves_as_it_was() {
    for name in UNSTAMPED {
        let bytes = fixture(name);
        let (one, two) = (editor(&bytes), editor(&bytes));
        let (one, two) = (one.store().snapshot(), two.store().snapshot());
        assert!(!one.doc().package().unwritten().is_empty(), "{name}: stamped at open");
        assert_eq!(ids(one.doc()), ids(two.doc()), "{name}");
        assert!(
            ids(one.doc())
                .iter()
                .flat_map(|(_, i)| i)
                .all(|id| id.contains(':') && !id.contains('#')),
            "{name}: {:?}",
            ids(one.doc())
        );
        assert!(
            *one.doc().package().save().unwrap() == *bytes,
            "{name}: a save before any edit"
        );
    }
}

#[test]
fn the_first_edit_writes_every_story_part_renames_nothing_and_reopens_with_the_same_ids() {
    for name in UNSTAMPED {
        let bytes = fixture(name);
        let shared = editor(&bytes);
        let opened = shared.store().snapshot();
        let before = ids(opened.doc());
        let target = paragraphs(opened.doc())[1].clone();
        let done = shared
            .run(&Caller::anonymous(), &set_text(&target, "Written."))
            .unwrap();
        assert!(done.changed, "{name}");
        assert!(done.version.doc().ids().aliases().is_empty(), "{name}");
        let doc = done.version.doc();
        assert!(doc.package().unwritten().is_empty(), "{name}: every part written");
        assert_eq!(ids(doc), before, "{name}: the edit kept every id");
        // saved and opened again: the same ids, none given at open
        let again = reopened(doc.package().save().unwrap());
        let again = again.store().snapshot();
        assert!(again.doc().package().unwritten().is_empty(), "{name}");
        assert_eq!(ids(again.doc()), before, "{name}: reopened");
    }
}

#[test]
fn an_undo_back_to_the_version_opened_saves_the_bytes_again() {
    for name in UNSTAMPED {
        let bytes = fixture(name);
        let shared = editor(&bytes);
        let target = paragraphs(shared.store().snapshot().doc())[0].clone();
        shared
            .run(&Caller::anonymous(), &set_text(&target, "Written."))
            .unwrap();
        let second = paragraphs(shared.store().snapshot().doc())[2].clone();
        shared.run(&Caller::anonymous(), &set_text(&second, "Again.")).unwrap();
        // one step back: still written, the ids as they were
        shared.store().undo().unwrap();
        let one_back = shared.store().snapshot();
        assert!(one_back.doc().package().unwritten().is_empty(), "{name}");
        shared.store().undo().unwrap();
        let back = shared.store().snapshot();
        assert!(
            *back.doc().package().save().unwrap() == *bytes,
            "{name}: undone to the version opened"
        );
        assert_eq!(
            ids(back.doc()),
            ids(one_back.doc()),
            "{name}: no id renamed by the undo"
        );
        // and redone: written again, the same ids
        shared.store().redo().unwrap();
        let redone = shared.store().snapshot();
        assert!(redone.doc().package().unwritten().is_empty(), "{name}");
        assert_eq!(ids(redone.doc()), ids(one_back.doc()), "{name}");
    }
}

#[test]
fn two_first_edits_made_at_once_both_commit() {
    let bytes = fixture("samplelib/sample-long.docx");
    let shared = editor(&bytes);
    let opened = shared.store().snapshot();
    let targets = paragraphs(opened.doc());
    let mut first = shared.store().begin(&Setup::default());
    let mut second = shared.store().begin(&Setup::default());
    shared
        .run_in(
            &mut first,
            &opened,
            &set_text(&targets[1], "First writer."),
            &Setup::default(),
        )
        .unwrap();
    shared
        .run_in(
            &mut second,
            &opened,
            &set_text(&targets[5], "Second writer."),
            &Setup::default(),
        )
        .unwrap();
    assert!(shared.store().commit(&Caller::named("one"), first).is_some());
    assert!(
        shared.store().commit(&Caller::named("two"), second).is_some(),
        "no conflict"
    );
    let head = shared.store().snapshot();
    assert!(head.doc().package().unwritten().is_empty());
    assert_eq!(ids(head.doc()), ids(opened.doc()));
    let body = head.doc().package().read("word/document.xml").unwrap().unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("First writer.") && body.contains("Second writer."));
}

#[test]
fn a_document_opened_with_its_id_state_before_any_edit_opens_again_with_the_same_ids() {
    let bytes = fixture("generated/toc-numbered.docx");
    let budget = || Arc::new(crate::common::Unlimited);
    let package = || ooxml_opc::Package::open(Arc::clone(&bytes), &ooxml_base::Limits::default()).unwrap();
    let doc = WordDoc::open(package(), budget()).unwrap();
    // the state's digest is the bytes' (the ids given at open are not written)
    let saved_roots: Vec<ooxml_xml::Tree> = doc
        .docx()
        .unwrap()
        .parts
        .iter()
        .filter_map(|p| doc.package().saved_tree(&p.part).unwrap())
        .collect();
    let roots: Vec<&ooxml_xml::Element> = saved_roots.iter().map(|t| &**t.root()).collect();
    let mut ledger = ooxml_word_model::ids::Ledger::default();
    ledger.note(&roots, 0);
    let state = ooxml_word_model::ids::export_state(
        &ledger,
        &roots,
        &ooxml_word_model::ids::Versions { version: 0, last: 0 },
        &|_| None,
    );
    let (again, _) = WordDoc::open_with_state(package(), budget(), &state).unwrap();
    assert_eq!(ids(&again), ids(&doc));
}

/// The `w14:textId` of the body paragraph `id` in `doc`, as read.
fn text_id(doc: &WordDoc, id: &str) -> String {
    let docx = doc.docx().unwrap();
    let body = docx.body().unwrap();
    let paragraph = body.ids.paragraphs().iter().find(|p| p.id == id).unwrap();
    let element = ooxml_word_model::docx::at_path(body.root(), &paragraph.path).unwrap();
    element
        .attribute(Some(ooxml_word_model::ids::W14), "textId")
        .unwrap()
        .to_owned()
}

#[test]
fn an_id_given_at_open_counts_as_fresh_at_its_blocks_first_write() {
    // the first text edit of a paragraph stamped at open keeps the pair it was given (as the
    // edit that stamped it kept the pair it drew); the next text edit renews its textId
    let bytes = fixture("samplelib/sample-long.docx");
    let shared = editor(&bytes);
    let target = paragraphs(shared.store().snapshot().doc())[1].clone();
    let given = text_id(shared.store().snapshot().doc(), &target);
    let first = shared
        .run(&Caller::anonymous(), &set_text(&target, "Written."))
        .unwrap();
    assert_eq!(text_id(first.version.doc(), &target), given, "the first write keeps it");
    let second = shared.run(&Caller::anonymous(), &set_text(&target, "Again.")).unwrap();
    assert_ne!(text_id(second.version.doc(), &target), given, "a later write renews it");
}

#[test]
fn an_id_never_written_stays_fresh_for_the_whole_transaction_that_first_writes_its_part() {
    // main's ruling (ST2, Q1): a textId renews against the version last written; one given at
    // open and never written stays whole through every write of the first transaction -- a
    // batch whose second op edits the block the first wrote keeps it -- and the next
    // transaction renews it
    let bytes = fixture("samplelib/sample-long.docx");
    let shared = editor(&bytes);
    let opened = shared.store().snapshot();
    let target = paragraphs(opened.doc())[1].clone();
    let given = text_id(opened.doc(), &target);
    let batch = ooxml_store::Batch::new(vec![
        set_text(&target, "Written once."),
        set_text(&target, "Written twice."),
    ]);
    let done = shared.run(&Caller::anonymous(), &batch).unwrap();
    assert!(done.changed);
    assert!(
        done.version.doc().package().unwritten().is_empty(),
        "every part written"
    );
    assert_eq!(text_id(done.version.doc(), &target), given, "kept through the batch");
    let next = shared
        .run(&Caller::anonymous(), &set_text(&target, "Written again."))
        .unwrap();
    assert_ne!(
        text_id(next.version.doc(), &target),
        given,
        "renewed by the next transaction"
    );
}

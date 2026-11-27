//! Every version a command makes through the editor has every paragraph and table row of every
//! story stamped (`stamping::Stamped`): Word drops every paraId of a document holding one row
//! without a paraId when it saves it (crates/ooxml-oracle/word/observations/
//! paraid-durability.json, factors "row"), so a command that stamps nothing itself -- cells
//! written as `_fill_cells` writes them -- still leaves the fixture's unstamped row stamped; the
//! same command run on the store alone (no editor) leaves it as it was. And what that costs per
//! edit on a long document: `STAMPED_BENCH=<path to a .docx> cargo test --release -p
//! ooxml-word-edit --test integration -- --ignored --nocapture stamped::`; and what the first
//! edit of an unstamped document costs (it stamps every block): `STAMPED_FIRST=<path to a
//! .docx>` (`STAMPED_ROUNDS`, 3 by default) with the same command.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture, written};
use ooxml_store::Caller;
use ooxml_word_edit::{CellAxis, CellWrite, RevisionPolicy, SetCells, SetText, WordDoc};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

/// The story parts' paragraphs and rows without a paraId, and the paraIds held twice.
fn unstamped(doc: &WordDoc) -> (usize, usize) {
    let (mut missing, mut repeated) = (0, 0);
    let mut seen = HashSet::new();
    let story = |n: &str| {
        n == "word/document.xml"
            || n.starts_with("word/header")
            || n.starts_with("word/footer")
            || ["word/footnotes.xml", "word/endnotes.xml", "word/comments.xml"].contains(&n)
    };
    for (name, bytes) in written(doc) {
        if !story(&name) {
            continue;
        }
        let xml = String::from_utf8(bytes.to_vec()).unwrap();
        for open in xml.split('<').skip(1) {
            let head = open.split('>').next().unwrap_or("");
            let tag = head.split([' ', '/']).next().unwrap_or("");
            if tag != "w:p" && tag != "w:tr" {
                continue;
            }
            match head.split("w14:paraId=\"").nth(1).and_then(|r| r.split('"').next()) {
                None => missing += 1,
                Some(id) if !seen.insert(id.to_owned()) => repeated += 1,
                Some(_) => {}
            }
        }
    }
    (missing, repeated)
}

fn fill(table: &str) -> SetCells {
    SetCells {
        table: ooxml_word_model::address::Address::from(table.to_owned()),
        writes: vec![CellWrite {
            row: CellAxis::At(0),
            column: CellAxis::At(0),
            text: "Written".to_owned(),
        }],
        stamping: false,
        policy: RevisionPolicy::Untracked,
        accepted: None,
    }
}

#[test]
fn a_command_that_stamps_nothing_leaves_every_block_stamped() {
    let bytes = fixture("generated/toc-numbered.docx");
    let shared = editor(&bytes);
    let before = shared.store().snapshot();
    assert_ne!(unstamped(before.doc()).0, 0, "the fixture has a row without a paraId");
    let docx = before.doc().docx().unwrap();
    let table = docx.parts[0].ids.tables().iter().next().unwrap().id.clone();

    // the store alone runs the command as it is: the row is left without its id
    let bare = editor(&bytes);
    let alone = bare.store().run(&Caller::anonymous(), &fill(&table)).unwrap();
    assert!(alone.changed);
    assert_ne!(unstamped(alone.version.doc()).0, 0);

    let done = shared.run(&Caller::anonymous(), &fill(&table)).unwrap();
    assert!(done.changed);
    assert_eq!(unstamped(done.version.doc()), (0, 0));
}

/// The median of `times`.
fn median(mut times: Vec<Duration>) -> Duration {
    times.sort_unstable();
    times[times.len() / 2]
}

#[test]
#[ignore = "a timing on a long document: STAMPED_BENCH=<docx> cargo test --release -- --ignored"]
fn the_commit_time_stamping_on_a_long_document() {
    let Ok(path) = std::env::var("STAMPED_BENCH") else {
        return;
    };
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let ids: Vec<String> = {
        let probe = editor(&bytes);
        let docx = probe.store().snapshot().doc().docx().unwrap();
        let body = docx.body().unwrap();
        body.ids
            .paragraphs()
            .iter()
            .filter(|p| p.path.len() == 2)
            .map(|p| p.id.clone())
            .collect()
    };
    let set = |k: usize| SetText {
        target: ooxml_word_model::address::Address::from(ids[ids.len() * k / 13 + 5].clone()),
        text: format!("Written by edit {k}."),
        policy: RevisionPolicy::Untracked,
    };
    let rounds = 9;
    // the store alone: the command's own commit; the editor: the records' follow-up and the
    // stamping after it as well. Each after a first edit (which stamps the whole document).
    let bare = editor(&bytes);
    bare.store().run(&Caller::anonymous(), &set(0)).unwrap();
    let alone: Vec<Duration> = (1..=rounds)
        .map(|k| bare.store().run(&Caller::anonymous(), &set(k)).unwrap().timing.total)
        .collect();
    let full = editor(&bytes);
    full.run(&Caller::anonymous(), &set(0)).unwrap();
    let through: Vec<Duration> = (1..=rounds)
        .map(|k| full.run(&Caller::anonymous(), &set(k)).unwrap().timing.commit.total)
        .collect();
    let (alone, through) = (median(alone), median(through));
    println!(
        "{} body paragraphs; set_text commit, median of {rounds}: the command alone {:.2} ms, with the \
         follow-up and the stamping {:.2} ms (+{:.2} ms)",
        ids.len(),
        alone.as_secs_f64() * 1000.0,
        through.as_secs_f64() * 1000.0,
        (through.as_secs_f64() - alone.as_secs_f64()) * 1000.0
    );
}

#[test]
#[ignore = "a timing on a long document: STAMPED_FIRST=<docx> cargo test --release -- --ignored"]
fn the_first_edit_of_an_unstamped_document() {
    let Ok(path) = std::env::var("STAMPED_FIRST") else {
        return;
    };
    let rounds: usize = std::env::var("STAMPED_ROUNDS").map_or(3, |r| r.parse().unwrap());
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let (blocks, target) = {
        let probe = editor(&bytes);
        let docx = probe.store().snapshot().doc().docx().unwrap();
        let blocks: usize = docx
            .parts
            .iter()
            .map(|p| p.ids.paragraphs().len() + p.ids.tables().iter().map(|t| t.rows.len()).sum::<usize>())
            .sum();
        let body = docx.body().unwrap();
        let ids: Vec<String> = body
            .ids
            .paragraphs()
            .iter()
            .filter(|p| p.path.len() == 2)
            .map(|p| p.id.clone())
            .collect();
        (blocks, ids[ids.len() / 2].clone())
    };
    let times: Vec<Duration> = (0..rounds)
        .map(|_| {
            let shared = editor(&bytes);
            let edit = SetText {
                target: ooxml_word_model::address::Address::from(target.clone()),
                text: "Written by the first edit.".to_owned(),
                policy: RevisionPolicy::Untracked,
            };
            let started = std::time::Instant::now();
            let done = shared.run(&Caller::anonymous(), &edit).unwrap();
            let took = started.elapsed();
            assert!(done.changed);
            assert_eq!(unstamped(done.version.doc()), (0, 0));
            took
        })
        .collect();
    let all: Vec<String> = times
        .iter()
        .map(|t| format!("{:.1}", t.as_secs_f64() * 1000.0))
        .collect();
    println!(
        "{blocks} blocks; first set_text, median of {rounds}: {:.1} ms (each: {} ms)",
        median(times).as_secs_f64() * 1000.0,
        all.join(", ")
    );
}

//! New ids drawn where the change lands (lane D1): two writers that each make new blocks
//! from one version both draw the same next ids while they prepare; drawn again where each
//! commit lands, they commit without a conflict, and the document and the ids each edit
//! reports equal a serial run in commit order. A stand-in written where no draw reaches is
//! pinned: the edit reads its seed's sequence, so it conflicts as every draw did before.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture, written};
use crate::interleave::Paused;
use ooxml_store::{Caller, Command, Txn};
use ooxml_word_edit::{
    EditError, EditSink, Edited, Editor, InsertMarkdown, Key, MarkdownInserted, MarkdownOptions, RevisionPolicy,
    Tracking, WordDoc, WriteMap,
};
use ooxml_xml::{Attribute, Element, Head, QName};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

const FIXTURE: &str = "generated/ids-and-markup.docx";

fn markdown(at: &str, text: &str, policy: RevisionPolicy) -> InsertMarkdown {
    InsertMarkdown {
        markdown: format!("## {text}\n\n- one {text}\n- two\n\n| a | b |\n|---|---|\n| {text} | c |\n"),
        at: at.into(),
        style_map: WriteMap::default(),
        options: MarkdownOptions::default(),
        policy,
    }
}

/// The body's top-level paragraphs, by id.
fn targets(shared: &Editor) -> Vec<String> {
    let docx = shared.store().snapshot().doc().docx().unwrap();
    docx.body()
        .unwrap()
        .ids
        .paragraphs()
        .iter()
        .filter(|p| p.path.len() == 2)
        .map(|p| p.id.clone())
        .collect()
}

/// `last` prepared, `between` committed, then `last` committed through the editor.
fn interleaved<C, B>(shared: &Editor, last: C, between: &B) -> (Edited<C::Out>, Edited<B::Out>)
where
    C: Command<WordDoc, Error = EditError> + Send + Sync,
    C::Out: Send + std::fmt::Debug,
    B: Command<WordDoc, Error = EditError>,
{
    let (prepared, wait) = channel();
    let (release, go) = channel();
    let paused = Paused {
        command: last,
        prepared,
        go: Mutex::new(go),
        first: AtomicBool::new(true),
    };
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| shared.run(&Caller::named("last"), &paused).unwrap());
        wait.recv().unwrap();
        let between = shared.run(&Caller::named("between"), between).unwrap();
        release.send(()).unwrap();
        (handle.join().unwrap(), between)
    })
}

/// The ids an insertion reports, named as drawn where it landed.
fn reported(done: &Edited<MarkdownInserted>) -> Vec<String> {
    done.out.created.iter().map(|id| done.drawn.resolve(id)).collect()
}

/// An editor of the fixture after a first insertion by `policy`, which stamps every block
/// (writing the whole part) and adds what the insertions after it use: the styles, and
/// tracked the people part and the root's declarations (real conflicts, not the ids').
fn opened(bytes: &Arc<[u8]>, policy: RevisionPolicy) -> Editor {
    let shared = editor(bytes);
    shared
        .run(&Caller::anonymous(), &markdown("end", "First", policy))
        .unwrap();
    shared
}

fn policy_of(command: &InsertMarkdown) -> RevisionPolicy {
    command.policy.clone()
}

fn two_insertions(policy: RevisionPolicy) {
    let bytes = fixture(FIXTURE);
    let shared = opened(&bytes, policy.clone());
    let at = targets(&shared);
    let first = markdown(&format!("after:{}", at[5]), "Between", policy.clone());
    let second = markdown(&format!("after:{}", at[1]), "Last", policy);
    let (last, between) = interleaved(&shared, second.clone(), &first);
    let stats = shared.store().stats();
    assert_eq!(
        (stats.conflicts, stats.exclusive),
        (0, 0),
        "drawing new ids is no conflict"
    );
    assert_eq!(between.attempts, 1);
    assert_eq!(last.attempts, 1);
    // both drew the same next ids where they prepared: the later one's moved
    assert!(
        last.drawn.moved().next().is_some(),
        "the second drew again where it landed"
    );
    // the serial run in commit order: the same bytes, the same ids reported
    let serial = opened(&bytes, policy_of(&first));
    let s_between = serial.run(&Caller::anonymous(), &first).unwrap();
    let s_last = serial.run(&Caller::anonymous(), &second).unwrap();
    assert_eq!(written(last.version.doc()), written(s_last.version.doc()));
    assert_eq!(reported(&between), s_between.out.created);
    assert_eq!(reported(&last), s_last.out.created);
    // the history keys the new blocks as drawn
    let steps = shared.store().steps();
    let made = steps.last().unwrap();
    for id in reported(&last).iter().filter(|id| id.starts_with("p:")) {
        let key = Key::block("word/document.xml", id.as_str());
        assert!(made.writes.contains(&key), "{id} kept as drawn in {:?}", made.writes);
    }
}

#[test]
fn two_insertions_from_one_version_commit_without_a_conflict() {
    two_insertions(RevisionPolicy::Untracked);
}

#[test]
fn two_tracked_insertions_from_one_version_commit_without_a_conflict() {
    two_insertions(RevisionPolicy::Tracked(Tracking {
        author: "Writer".to_owned(),
        date: "2026-11-17T10:00:00Z".to_owned(),
    }));
}

/// A stand-in drawn and written into the first body paragraph's `attribute` (a copy of the
/// paragraph replacing it): whether the edit read its seed's sequence (pinned).
struct Written {
    attribute: QName,
}

impl Command<WordDoc> for Written {
    type Out = bool;
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<(), EditError> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<bool, EditError> {
        let docx = tx.doc().docx()?;
        let body = docx.body().unwrap();
        let paragraph = body.ids.paragraphs().first().unwrap();
        let path = paragraph.path.clone();
        let old = ooxml_word_model::docx::at_path(body.root(), &path).unwrap().clone();
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        let drawn = sink.draw("probe", 2)?;
        let mut attributes: Vec<Attribute> = old
            .attributes()
            .iter()
            .filter(|a| a.name.as_ref() != &self.attribute)
            .cloned()
            .collect();
        attributes.push(Attribute {
            name: Arc::new(self.attribute.clone()),
            value: Arc::from(drawn[0].as_str()),
        });
        let head = Head {
            name: old.name().clone(),
            namespaces: old.namespaces().to_vec().into(),
            attributes: attributes.into(),
        };
        let new = Arc::new(Element::from_head(Arc::new(head), old.children().clone()));
        let key = Key::block(docx.main.as_str(), paragraph.id.as_str());
        sink.replace(&docx.main, &path, new, vec![key])?;
        Ok(tx.reads().contains(&Key::Seq(Arc::from("probe"))))
    }
}

const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

#[test]
fn a_stand_in_in_an_id_attribute_is_drawn_where_it_lands() {
    let shared = editor(&fixture(FIXTURE));
    let pinned = shared
        .run(
            &Caller::anonymous(),
            &Written {
                attribute: QName::new(Some("w14"), "textId", Some(W14)),
            },
        )
        .unwrap()
        .out;
    assert!(!pinned, "a draw reaches it: its sequence is not read");
}

#[test]
fn a_stand_in_no_draw_reaches_is_pinned() {
    let shared = editor(&fixture(FIXTURE));
    let pinned = shared
        .run(
            &Caller::anonymous(),
            &Written {
                attribute: QName::new(Some("w"), "rsidR", Some(W)),
            },
        )
        .unwrap()
        .out;
    assert!(pinned, "an attribute the scan does not cover pins the stand-in");
}

/// Writers each inserting Markdown after paragraphs of their own, concurrently: every
/// version and every id an insertion reports equal the serial run in commit order, and
/// drawing the new ids conflicts nowhere.
#[test]
fn concurrent_insertions_equal_their_serial_order() {
    for seed in [3u64, 4] {
        let bytes = fixture(FIXTURE);
        let shared = opened(&bytes, RevisionPolicy::Untracked);
        let at = targets(&shared);
        let writers = 4usize;
        let mut records: Vec<(ooxml_store::Seq, InsertMarkdown, Edited<MarkdownInserted>)> = Vec::new();
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..writers)
                .map(|w| {
                    let (shared, at) = (&shared, &at);
                    scope.spawn(move || {
                        let caller = Caller::named(&format!("writer {w}"));
                        (0..3)
                            .map(|n| {
                                let target = &at[(w * 2 + n * 5 + seed as usize) % at.len()];
                                let policy = if (w + n) % 2 == 0 {
                                    RevisionPolicy::Untracked
                                } else {
                                    RevisionPolicy::Tracked(Tracking {
                                        author: format!("Writer {w}"),
                                        date: "2026-11-17T10:00:00Z".to_owned(),
                                    })
                                };
                                let command = markdown(&format!("after:{target}"), &format!("w{w}n{n}"), policy);
                                let done = shared.run(&caller, &command).unwrap();
                                (done.version.seq(), command, done)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for handle in handles {
                records.extend(handle.join().unwrap());
            }
        });
        records.sort_by_key(|r| r.0);
        let serial = opened(&bytes, RevisionPolicy::Untracked);
        for (at, command, done) in &records {
            let s = serial.run(&Caller::anonymous(), command).unwrap();
            assert_eq!(s.version.seq(), *at, "seed {seed}: commit order");
            assert_eq!(
                written(s.version.doc()),
                written(done.version.doc()),
                "seed {seed}: version {at}"
            );
            assert_eq!(reported(done), s.out.created, "seed {seed}: ids reported at {at}");
        }
    }
}

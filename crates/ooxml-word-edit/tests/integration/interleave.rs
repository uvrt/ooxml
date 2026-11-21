//! Two writers interleaved by hand (lane P2c15, shared with lane P2c18's review reads): one
//! edit prepared and held, another committed meanwhile, then the first committed -- to see
//! what an edit's reads make a conflict.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::{Caller, Command, Txn};
use ooxml_word_edit::{EditError, Editor, RevisionPolicy, SetText, WordDoc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

/// `command`, held after its first prepare until told to go on.
pub(crate) struct Paused<C> {
    pub(crate) command: C,
    pub(crate) prepared: Sender<()>,
    pub(crate) go: Mutex<Receiver<()>>,
    pub(crate) first: AtomicBool,
}

impl<C: Command<WordDoc, Error = EditError>> Command<WordDoc> for Paused<C> {
    type Out = C::Out;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        self.command.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<C::Out, EditError> {
        let out = self.command.prepare(tx)?;
        if self.first.swap(false, Ordering::SeqCst) {
            self.prepared.send(()).unwrap();
            self.go.lock().unwrap().recv().unwrap();
        }
        Ok(out)
    }
}

/// An editor of the fixture after a first edit (which stamps every block, writing the
/// whole part), and the body's top-level paragraphs by their stamped ids.
pub fn opened() -> (Editor, Vec<String>) {
    let shared = editor(&fixture("generated/ids-and-markup.docx"));
    let ids = |shared: &Editor| -> Vec<String> {
        let docx = shared.store().snapshot().doc().docx().unwrap();
        let body = docx.body().unwrap();
        body.ids
            .paragraphs()
            .iter()
            .filter(|p| p.path.len() == 2)
            .map(|p| p.id.clone())
            .collect()
    };
    let first = ids(&shared).last().cloned().unwrap();
    shared.store().run(&Caller::anonymous(), &set_text(&first)).unwrap();
    let targets = ids(&shared);
    assert!(targets.len() >= 8, "{targets:?}");
    (shared, targets)
}

/// `last` prepared, `between` committed, then `last` committed: whether `last` succeeded,
/// and the store's conflicts.
pub fn interleaved<C, B>(shared: &Editor, last: C, between: &B) -> (bool, u64)
where
    C: Command<WordDoc, Error = EditError> + Send + Sync,
    C::Out: Send,
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
    let done = std::thread::scope(|scope| {
        let handle = scope.spawn(|| shared.store().run(&Caller::named("last"), &paused).is_ok());
        wait.recv().unwrap();
        shared.store().run(&Caller::named("between"), between).unwrap();
        release.send(()).unwrap();
        handle.join().unwrap()
    });
    (done, shared.store().stats().conflicts)
}

pub fn set_text(target: &str) -> SetText {
    SetText {
        target: target.to_owned(),
        text: "Written meanwhile".to_owned(),
        policy: RevisionPolicy::Untracked,
    }
}

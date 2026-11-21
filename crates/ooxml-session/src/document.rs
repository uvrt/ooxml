//! Owns: one open document as the session holds it ([`Document`]): its kind and name, its
//! versions in a store over the model its kind is kept as -- a Word document's in its
//! editor over `ooxml-word-edit`'s [`WordDoc`] (phase P2: Rust owns it), any other kind's
//! as its package ([`PackageDoc`]) --, a version of either as a call reads it ([`At`]), and
//! what the host keeps of a Word document beside its versions: the version it was opened as
//! (what a session's own edits are told from) and its tracking mode (each later edit's
//! revision policy), the ids its versions retired and the rows its table calls wrote.
//!
//! Must not: dispatch calls or hold the session's table; parse or write a package beyond
//! opening it; read the clock but through the store's port.

use crate::context::StoreClock;
use crate::model::PackageDoc;
use crate::retired::{Made, Retired};
use crate::update::PartUpdate;
use crate::{DocKind, SessionError};
use ooxml_base::Context;
use ooxml_opc::Package;
use ooxml_store::{Caller, Committed, Snapshot, Store, StoreConfig, StoreStats};
use ooxml_tools_word::{RowsWritten, Tracked};
use ooxml_word_edit::validate::Delta;
use ooxml_word_edit::{EarlierIds, Editor, RevisionPolicy, Tracking, WordDoc};
use ooxml_word_model::docx::Versions;
use std::sync::{Arc, PoisonError, RwLock};

/// The context's cache budget, as the edit core's caches reserve from it.
struct Budget(Arc<dyn ooxml_base::CacheBudget>);

impl ooxml_derive::CacheBudget for Budget {
    fn reserve(&self, bytes: usize) -> bool {
        self.0.reserve(bytes)
    }

    fn release(&self, bytes: usize) {
        self.0.release(bytes);
    }

    fn used(&self) -> usize {
        self.0.used()
    }
}

/// A Word document's versions and what the host keeps of it.
pub(crate) struct Word {
    /// Its versions, and the problems it opened with.
    pub(crate) editor: Editor,
    /// The version it was opened as.
    pub(crate) opened: Snapshot<WordDoc>,
    /// Who its edits are tracked as (`None`: untracked), as `word_set_tracking` left it.
    tracking: RwLock<Option<String>>,
    /// The ids its versions removed, and who removed each.
    pub(crate) retired: Retired,
    /// The rows this session's `word_edit_table` calls added or wrote into (round 58,
    /// R126): `check`'s `row_text_format` reads them.
    pub(crate) rows_written: RowsWritten,
}

impl Word {
    /// The author edits are tracked as, if any.
    pub(crate) fn tracking(&self) -> Option<String> {
        self.tracking.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Keep the mode `word_set_tracking` left.
    pub(crate) fn set_tracking(&self, mode: Tracked) {
        let author = match mode {
            Tracked::On(author) => Some(author),
            Tracked::Off => None,
        };
        *self.tracking.write().unwrap_or_else(PoisonError::into_inner) = author;
    }

    /// `version`'s problems against those the document opened with: what the save gate
    /// compares.
    pub(crate) fn validation(&self, version: &Snapshot<WordDoc>) -> Result<Delta, SessionError> {
        let now = ooxml_word_edit::validate::problems(version.doc())?;
        Ok(Delta::of(self.editor.baseline(), &now))
    }

    /// The revision policy an edit made now is written under: tracked as the mode's
    /// author, dated `date` (ISO 8601, the session's clock), or untracked.
    pub(crate) fn policy(&self, date: &str) -> RevisionPolicy {
        match self.tracking() {
            Some(author) => RevisionPolicy::Tracked(Tracking {
                author,
                date: date.to_owned(),
            }),
            None => RevisionPolicy::Untracked,
        }
    }
}

/// A document's versions, by the model its kind is kept as.
pub(crate) enum Held {
    /// A package (kinds Rust does not own yet).
    Package(Box<Store<PackageDoc>>),
    /// A Word document.
    Word(Box<Word>),
}

/// One version of a document, as a call reads it.
#[derive(Clone)]
pub(crate) enum At {
    /// A package's.
    Package(Snapshot<PackageDoc>),
    /// A Word document's.
    Word(Snapshot<WordDoc>),
}

impl At {
    /// The version's number.
    pub(crate) fn seq(&self) -> u64 {
        match self {
            At::Package(v) => v.seq().0,
            At::Word(v) => v.seq().0,
        }
    }

    /// The package at this version.
    pub(crate) fn package(&self) -> &Package {
        match self {
            At::Package(v) => v.doc().package(),
            At::Word(v) => v.doc().package(),
        }
    }
}

/// One open document.
pub(crate) struct Document {
    pub(crate) kind: DocKind,
    pub(crate) name: String,
    pub(crate) held: Held,
    /// Its readings as a Word document, kept for its latest versions (design section 7:
    /// caches belong to a document).
    pub(crate) versions: Arc<Versions>,
}

impl Document {
    /// `package` opened as `kind`: a Word document read, its ids (with those it had before
    /// it was saved, `earlier`) and the problems it opened with found, its caches within the
    /// context's budget.
    pub(crate) fn open(
        package: Package,
        kind: DocKind,
        name: &str,
        config: StoreConfig,
        context: &Context,
        earlier: &EarlierIds,
    ) -> Result<Document, SessionError> {
        let clock = Arc::new(StoreClock(Arc::clone(&context.clock)));
        let held = match kind {
            DocKind::Docx => {
                let budget = Arc::new(Budget(Arc::clone(&context.budget)));
                let editor = Editor::open_with(package, config, clock, budget, earlier)?;
                let opened = editor.store().snapshot();
                // the paragraph texts every find reads and the facts the document-wide
                // lookups read (bookmarks, controls, revisions, frames), read once as the
                // document opens: each later version reads only the blocks it changed, and
                // no edit call pays for the first reading of a long part
                for part in &opened.doc().docx()?.parts {
                    part.texts();
                    part.facts();
                }
                Held::Word(Box::new(Word {
                    editor,
                    opened,
                    tracking: RwLock::new(None),
                    retired: Retired::default(),
                    rows_written: RowsWritten::default(),
                }))
            }
            DocKind::Pptx | DocKind::Xlsx => {
                Held::Package(Box::new(Store::new(PackageDoc::new(package), config, clock)))
            }
        };
        Ok(Document {
            kind,
            name: name.to_owned(),
            held,
            versions: Arc::new(Versions::new()),
        })
    }

    /// The head version.
    pub(crate) fn head(&self) -> At {
        match &self.held {
            Held::Package(store) => At::Package(store.snapshot()),
            Held::Word(word) => At::Word(word.editor.store().snapshot()),
        }
    }

    /// The Word document's state, if it is one.
    pub(crate) fn word(&self) -> Option<&Word> {
        match &self.held {
            Held::Word(word) => Some(word),
            Held::Package(_) => None,
        }
    }

    /// The store's counters.
    pub(crate) fn stats(&self) -> StoreStats {
        match &self.held {
            Held::Package(store) => store.stats(),
            Held::Word(word) => word.editor.store().stats(),
        }
    }

    /// Commit the bridge's update as `caller`, made by `tool`: the version it leaves,
    /// whether it changed anything, and how many times it was prepared. A Word document's
    /// retired ids note what it removed.
    pub(crate) fn update(
        &self,
        caller: &Caller,
        tool: Option<&str>,
        update: &PartUpdate<'_>,
    ) -> Result<(At, bool, u32), SessionError> {
        fn out<D: ooxml_store::DocModel>(done: Committed<D, ()>, at: fn(Snapshot<D>) -> At) -> (At, bool, u32) {
            (at(done.version), done.changed, done.attempts)
        }
        Ok(match &self.held {
            Held::Package(store) => out(store.run(caller, update)?, At::Package),
            Held::Word(word) => {
                let done = word.editor.store().run(caller, update)?;
                if let Some(previous) = done.previous.as_ref().filter(|_| done.changed) {
                    let delta = ooxml_word_edit::ids_delta(previous.doc(), done.version.doc())?;
                    let made = Made {
                        seq: done.version.seq().0,
                        caller: caller.name(),
                        tool,
                        history: false,
                    };
                    word.retired.note(&made, &delta);
                }
                out(done, At::Word)
            }
        })
    }
}

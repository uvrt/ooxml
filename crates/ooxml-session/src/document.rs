//! Owns: one open document as the session holds it ([`Document`]): its kind and name, and
//! its format's [`FormatDoc`] -- a Word document's versions in its editor over
//! `ooxml-word-edit`'s [`WordDoc`] (phase P2: Rust owns it), any other kind's as its package
//! ([`PackageDoc`]), each a [`Stored`] -- and what the host keeps of a Word document beside
//! its versions: the version it was opened as
//! (what a session's own edits are told from) and its tracking mode (each later edit's
//! revision policy), the ids its versions retired, the rows its table calls wrote and its
//! page engine where the session lays it out.
//!
//! Must not: dispatch calls or hold the session's table; parse or write a package beyond
//! opening it; read the clock but through the store's port.

use crate::context::StoreClock;
use crate::format::{AnyVersion, Format, FormatDoc};
use crate::model::PackageDoc;
use crate::owners::{Owners, StepOwner};
use crate::retired::{Made, Retired};
use crate::stored::{Holds, StepBy, Stored, StoredDoc};
use crate::update::PartUpdate;
use crate::{DocKind, SessionError};
use ooxml_base::Context;
use ooxml_opc::Package;
use ooxml_store::{Caller, Snapshot, Store, StoreConfig};
use ooxml_tools_word::{MadeBy, RowsWritten};
use ooxml_word_edit::validate::Delta;
use ooxml_word_edit::{EarlierIds, Editor, WordDoc};
use ooxml_word_model::docx::Versions;
use ooxml_word_model::ids::LoadedState;
use std::any::Any;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// The context's cache budget, as the edit core's caches reserve from it.
pub(crate) struct Budget(pub(crate) Arc<dyn ooxml_base::CacheBudget>);

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

/// Word documents (`docx`).
#[derive(Debug, Clone, Copy, Default)]
pub struct WordFormat;

impl Format for WordFormat {
    fn kind(&self) -> DocKind {
        DocKind::Docx
    }

    fn register(&self, tools: &mut crate::Registry) {
        ooxml_tools_word::register(&mut tools.registrar(self.kind()));
    }
}

/// What holds a Word document's store: its editor, and the ids its versions retired, noted
/// as the versions are made and numbered.
pub(crate) struct WordStore {
    /// Its versions, and the problems it opened with.
    pub(crate) editor: Editor,
    /// The ids its versions removed, and who removed each.
    pub(crate) retired: Retired,
}

impl Holds<WordDoc> for WordStore {
    fn store(&self) -> &Store<WordDoc> {
        self.editor.store()
    }

    fn package(doc: &WordDoc) -> &Package {
        doc.package()
    }

    fn noted(
        &self,
        previous: &Snapshot<WordDoc>,
        version: &Snapshot<WordDoc>,
        by: StepBy<'_>,
    ) -> Result<(), SessionError> {
        let delta = ooxml_word_edit::ids_delta(previous.doc(), version.doc())?;
        let made = Made {
            seq: version.seq().0,
            caller: by.caller,
            tool: by.tool,
            history: by.history,
        };
        self.retired.note(&made, &delta);
        Ok(())
    }

    fn numbered(&self, seq: u64, number: u64) {
        self.retired.number(seq, number);
    }
}

/// A Word document's versions and what the host keeps of it.
pub(crate) struct Word {
    /// Its versions, history and step owners, over its editor's store.
    pub(crate) stored: Stored<WordDoc, WordStore>,
    /// The version it was opened as.
    pub(crate) opened: Snapshot<WordDoc>,
    /// Who its edits are tracked as (`None`: untracked), as `word_set_tracking` left it: the
    /// session's alone (no host keeps a copy). An edit reads it for its whole call, so a
    /// change of it never falls between an edit's prepare and its commit.
    tracking: RwLock<Option<String>>,
    /// What its last check with `reflow` saw.
    pub(crate) last_check: Arc<ooxml_tools_word::LastCheck>,
    /// Its pages drawn by `render`, kept within the session's memory budget.
    pub(crate) images: ooxml_tools_word::PageImages,
    /// The rows this session's `word_edit_table` calls added or wrote into (round 58,
    /// R126): `check`'s `row_text_format` reads them.
    pub(crate) rows_written: RowsWritten,
    /// The calls of this session that left a table ragged or off its grid (rounds 57 and
    /// 58): `check`'s `table_grid_mismatch` names them (`made_by`).
    pub(crate) made_by: MadeBy,
    /// Its page engine, where the session lays it out itself, and what its host set it to
    /// be laid out with (`crate::pages`).
    pub(crate) pages: crate::pages::DocPages,
    /// The versions its live views were given, kept within the session's memory budget.
    pub(crate) views: crate::live_view::LiveViews,
}

impl Word {
    /// Its editor.
    pub(crate) fn editor(&self) -> &Editor {
        &self.stored.held().editor
    }

    /// The ids its versions removed, and who removed each.
    pub(crate) fn retired(&self) -> &Retired {
        &self.stored.held().retired
    }

    /// The author edits are tracked as now, if any.
    pub(crate) fn tracking(&self) -> Option<String> {
        self.tracking_read().clone()
    }

    /// The tracking mode, held for an edit's whole call: `word_set_tracking` and a step
    /// group's revert wait until the edits holding it have committed. (A panic while it is
    /// held leaves the plain value whole: the poison is ignored.)
    pub(crate) fn tracking_read(&self) -> RwLockReadGuard<'_, Option<String>> {
        self.tracking.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// The tracking mode, held alone (`word_set_tracking`'s whole call).
    pub(crate) fn tracking_write(&self) -> RwLockWriteGuard<'_, Option<String>> {
        self.tracking.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// A version of it, erased.
    pub(crate) fn any(version: Snapshot<WordDoc>) -> AnyVersion {
        Stored::<WordDoc, WordStore>::any(version)
    }

    /// The head, or for the thread whose batch holds the document the version its ops left.
    pub(crate) fn head(&self) -> Snapshot<WordDoc> {
        self.stored.head()
    }

    /// The owner of the batch this thread holds on the document, if it holds one: its
    /// calls' edits run in the batch's transaction.
    pub(crate) fn batch_owner(&self) -> Option<ooxml_store::TxnOwner> {
        self.stored.batch_owner()
    }

    /// The tracking author a step group's revert goes back to (`None`: untracked).
    pub(crate) fn restore_tracking(&self, author: Option<String>) {
        *self.tracking_write() = author;
    }

    /// `version`'s problems against those the document opened with: what the save gate
    /// compares.
    pub(crate) fn validation(&self, version: &Snapshot<WordDoc>) -> Result<Delta, SessionError> {
        let now = ooxml_word_edit::validate::problems(version.doc())?;
        Ok(Delta::of(self.editor().baseline(), &now))
    }
}

impl FormatDoc for Word {
    fn stored(&self) -> &dyn StoredDoc {
        &self.stored
    }

    fn save(&self, at: &AnyVersion, checked: bool) -> Result<Vec<u8>, SessionError> {
        if let (true, Some(version)) = (checked, at.snapshot::<WordDoc>()) {
            let delta = self.validation(version)?;
            if !delta.new.is_empty() {
                return Err(SessionError::NewProblems { new: delta.new });
            }
        }
        Ok(at.package().save()?)
    }

    fn update(
        &self,
        caller: &Caller,
        tool: Option<&str>,
        update: &PartUpdate<'_>,
    ) -> Result<(AnyVersion, bool, u32, bool), SessionError> {
        let (version, changed, attempts, batch) = self.stored.update(caller, tool, update)?;
        Ok((Self::any(version), changed, attempts, batch))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A package's versions (kinds Rust does not own yet).
impl FormatDoc for Stored<PackageDoc> {
    fn stored(&self) -> &dyn StoredDoc {
        self
    }

    fn save(&self, at: &AnyVersion, _checked: bool) -> Result<Vec<u8>, SessionError> {
        Ok(at.package().save()?)
    }

    fn update(
        &self,
        caller: &Caller,
        tool: Option<&str>,
        update: &PartUpdate<'_>,
    ) -> Result<(AnyVersion, bool, u32, bool), SessionError> {
        let (version, changed, attempts, batch) = Stored::update(self, caller, tool, update)?;
        Ok((Self::any(version), changed, attempts, batch))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// One open document.
pub(crate) struct Document {
    pub(crate) kind: DocKind,
    pub(crate) name: String,
    /// Its versions and its format's state of it.
    pub(crate) doc: Box<dyn FormatDoc>,
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
        let doc: Box<dyn FormatDoc> = match kind {
            DocKind::Docx => {
                let budget = Arc::new(Budget(Arc::clone(&context.budget)));
                let editor = Editor::open_with(package, config, clock, budget, earlier)?;
                Box::new(Self::word_of(editor, context)?)
            }
            DocKind::Pptx | DocKind::Xlsx => Box::new(Stored::new(Store::new(PackageDoc::new(package), config, clock))),
        };
        Ok(Self::of(kind, name, doc))
    }

    /// `package` opened as a Word document with the id state saved beside its bytes
    /// (`docx-ids.1.`), read against its parts as they are parsed: its ids those it had
    /// before it was saved; and what the state recorded (its versions, the ids it retired).
    pub(crate) fn open_word_state(
        package: Package,
        name: &str,
        config: StoreConfig,
        context: &Context,
        state: &str,
    ) -> Result<(Document, LoadedState), SessionError> {
        let clock = Arc::new(StoreClock(Arc::clone(&context.clock)));
        let budget = Arc::new(Budget(Arc::clone(&context.budget)));
        let (editor, loaded) = Editor::open_with_state(package, config, clock, budget, state)?;
        let word = Self::word_of(editor, context)?;
        Ok((Self::of(DocKind::Docx, name, Box::new(word)), loaded))
    }

    fn of(kind: DocKind, name: &str, doc: Box<dyn FormatDoc>) -> Document {
        Document {
            kind,
            name: name.to_owned(),
            doc,
            versions: Arc::new(Versions::new()),
        }
    }

    /// A Word document's state as the session holds it, from its editor.
    fn word_of(editor: Editor, context: &Context) -> Result<Word, SessionError> {
        let opened = editor.store().snapshot();
        // the paragraph texts every find reads and the facts the document-wide
        // lookups read (bookmarks, controls, revisions, frames), read once as the
        // document opens: each later version reads only the blocks it changed, and
        // no edit call pays for the first reading of a long part
        for part in &opened.doc().docx()?.parts {
            part.texts();
            part.facts();
        }
        Ok(Word {
            stored: Stored::new(WordStore {
                editor,
                retired: Retired::default(),
            }),
            opened,
            tracking: RwLock::new(None),
            last_check: Arc::new(ooxml_tools_word::LastCheck::with_languages(
                ooxml_word_edit::facts::Detections::new(Arc::new(Budget(Arc::clone(&context.budget)))),
            )),
            images: ooxml_tools_word::PageImages::new(Some(Arc::clone(&context.budget))),
            rows_written: RowsWritten::default(),
            made_by: MadeBy::default(),
            pages: crate::pages::DocPages::default(),
            views: crate::live_view::LiveViews::new(Arc::clone(&context.budget)),
        })
    }

    /// Its versions and history.
    pub(crate) fn stored(&self) -> &dyn StoredDoc {
        self.doc.stored()
    }

    /// The head version; for the thread whose batch holds the document, the version its
    /// ops left ([`crate::group`]).
    pub(crate) fn head(&self) -> AnyVersion {
        self.stored().head()
    }

    /// The Word document's state, if it is one.
    pub(crate) fn word(&self) -> Option<&Word> {
        self.doc.as_any().downcast_ref()
    }

    /// Who made each of its steps, and its callers' own undos.
    pub(crate) fn owners(&self) -> &Owners {
        self.stored().owners()
    }

    /// A step numbered `number`, ended by the version `seq`, was made by `tool` (`None`:
    /// none, a change from outside the tools) as `context`'s caller: kept as its owner, and
    /// its number as a refusal of an id it retired names it.
    pub(crate) fn stepped(&self, seq: u64, number: u64, tool: Option<&str>, context: &crate::CallContext) {
        let owner = tool.map(|tool| StepOwner::of(tool, context));
        self.stored().stepped(seq, number, owner);
    }

    /// Commit a test's update of parts as `caller`, made by `tool` ([`FormatDoc::update`]).
    pub(crate) fn update(
        &self,
        caller: &Caller,
        tool: Option<&str>,
        update: &PartUpdate<'_>,
    ) -> Result<(AnyVersion, bool, u32, bool), SessionError> {
        self.doc.update(caller, tool, update)
    }
}

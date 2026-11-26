//! Owns: Word's format (design phase 3, section 3): [`WordFormat`], how a Word document
//! opens -- read by the edit core, its ids (with those it had before it was saved), the
//! problems it opened with -- with its tools; and one open Word document as the session
//! holds it ([`Word`], its [`FormatDoc`]): its versions in its editor over `ooxml-word-edit`'s
//! [`WordDoc`], in a [`Stored`] through [`WordStore`], and what the host keeps of it beside
//! its versions: the version it was opened as (what a session's own edits are told from),
//! its tracking mode (each later edit's revision policy), the ids its versions retired, the
//! rows its table calls wrote, its readings and its page engine where the session lays it
//! out; how a tool call runs on it ([`crate::call`]) and what a changing call made
//! ([`WordMade`]).
//!
//! Must not: dispatch calls or hold the session's table; parse or write a package beyond
//! opening it; read the clock but through the store's port.

use crate::call::{Call, WordHost};
use crate::retired::{Made as RetiredMade, Retired};
use crate::WordVersion;
use ooxml_base::Context;
use ooxml_opc::Package;
use ooxml_session::{
    AnyVersion, Finished, Format, FormatDoc, FormatRun, Holds, LayoutCancel, OpenEnv, PartUpdate, Registry,
    SessionError, StepBy, StoreClock, Stored, StoredDoc,
};
use ooxml_store::{Caller, Snapshot, Store};
use ooxml_tools::{CallEnv, Outcome, Tool, ToolResult};
use ooxml_tools_word::{LayoutLimit, Made, MadeBy, RowsWritten, WordLayout};
use ooxml_word_edit::validate::Delta;
use ooxml_word_edit::{Change, EarlierIds, Editor, IdsChange, WordDoc};
use ooxml_word_model::docx::Versions;
use ooxml_word_model::ids::LoadedState;
use ooxml_xml::Tree;
use serde_json::Value;
use std::any::Any;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

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

/// Word documents (`docx`), laid out by the session with `pages` where given.
#[derive(Debug, Clone, Default)]
pub struct WordFormat {
    /// What the session lays its Word documents out with, each document's engine its own
    /// (fonts and memory budget); `None`: only with the layout a call's host gives.
    pub pages: Option<crate::SessionPages>,
}

impl Format for WordFormat {
    fn kind(&self) -> ooxml_session::DocKind {
        ooxml_session::DocKind::Docx
    }

    fn register(&self, tools: &mut Registry) {
        ooxml_tools_word::register(&mut tools.registrar(self.kind()));
    }

    fn host_facts(&self) -> Vec<(String, ooxml_tools::roles::HostFacts)> {
        ooxml_tools_word::host_facts()
    }

    fn tool_hints(&self) -> Vec<(String, String)> {
        pairs(&ooxml_tools_word::TOOL_HINTS)
    }

    fn withdrawn(&self) -> Vec<(String, String)> {
        pairs(&ooxml_tools_word::WITHDRAWN)
    }

    fn open(&self, package: Package, env: &OpenEnv<'_>) -> Result<Arc<dyn FormatDoc>, SessionError> {
        let word = Word::open(package, env, &EarlierIds::default(), self.pages.clone())?;
        Ok(Arc::new(word))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn pairs(table: &[(&str, &str)]) -> Vec<(String, String)> {
    table.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
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

    fn put_tree(part: String, tree: Tree) -> Change {
        Change::Put {
            part: Arc::from(part),
            tree,
        }
    }

    fn put_bytes(part: String, bytes: Arc<[u8]>) -> Change {
        Change::Bytes {
            part: Arc::from(part),
            bytes,
        }
    }

    fn remove(part: String) -> Change {
        Change::Remove { part: Arc::from(part) }
    }

    fn ledger(doc: &WordDoc, trees: &[&Tree]) -> Option<Change> {
        let roots: Vec<&ooxml_xml::Element> = trees.iter().map(|t| &**t.root()).collect();
        let mut values: Vec<u32> = ooxml_word_model::ids::used_long_hex(&roots)
            .into_iter()
            .filter(|v| !doc.ids().contains(*v))
            .collect();
        values.sort_unstable();
        // a drawing the parts bring is one the document had (a copy keeps no number it had),
        // and raises the highest number given (never given again)
        let mut drawings: Vec<u64> = ooxml_word_model::ids::drawing_numbers(&roots)
            .into_iter()
            .filter(|n| !doc.ids().had_drawing(*n))
            .collect();
        drawings.sort_unstable();
        let highest = drawings
            .last()
            .copied()
            .filter(|n| *n > doc.ids().highest_drawing())
            .unwrap_or(0);
        let adds = !values.is_empty() || !drawings.is_empty();
        adds.then(|| {
            Change::Ids(IdsChange {
                values,
                highest_drawing: highest,
                drawings,
                ..IdsChange::default()
            })
        })
    }

    fn noted(
        &self,
        previous: &Snapshot<WordDoc>,
        version: &Snapshot<WordDoc>,
        by: StepBy<'_>,
    ) -> Result<(), SessionError> {
        let delta = ooxml_word_edit::ids_delta(previous.doc(), version.doc()).map_err(SessionError::format)?;
        let made = RetiredMade {
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
pub struct Word {
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
    /// Its readings as a Word document, kept for its latest versions (design section 7:
    /// caches belong to a document).
    pub(crate) versions: Arc<Versions>,
    /// What the session lays it out with by default ([`WordFormat::pages`]).
    pub(crate) default_pages: Option<crate::SessionPages>,
}

impl Word {
    /// `package` opened as a Word document: read, its ids (with those it had before it was
    /// saved, `earlier`) and the problems it opened with found, its caches within the
    /// context's budget.
    ///
    /// # Errors
    /// The edit core's, when the package is not a Word document it can read.
    pub fn open(
        package: Package,
        env: &OpenEnv<'_>,
        earlier: &EarlierIds,
        pages: Option<crate::SessionPages>,
    ) -> Result<Word, SessionError> {
        let (clock, budget) = ports(env.context);
        let editor = Editor::open_with(package, env.store, clock, budget, earlier).map_err(SessionError::format)?;
        Self::of_editor(editor, env.context, pages)
    }

    /// `package` opened as a Word document with the id state saved beside its bytes
    /// (`docx-ids.1.`), read against its parts as they are parsed: its ids those it had
    /// before it was saved; and what the state recorded (its versions, the ids it retired).
    ///
    /// # Errors
    /// The edit core's ([`ooxml_word_edit::EditError::IdState`] when the state is not these
    /// bytes').
    pub fn open_state(
        package: Package,
        env: &OpenEnv<'_>,
        state: &str,
        pages: Option<crate::SessionPages>,
    ) -> Result<(Word, LoadedState), SessionError> {
        let (clock, budget) = ports(env.context);
        let (editor, loaded) =
            Editor::open_with_state(package, env.store, clock, budget, state).map_err(SessionError::format)?;
        Ok((Self::of_editor(editor, env.context, pages)?, loaded))
    }

    /// A Word document's state as the session holds it, from its editor.
    fn of_editor(editor: Editor, context: &Context, pages: Option<crate::SessionPages>) -> Result<Word, SessionError> {
        let opened = editor.store().snapshot();
        // the paragraph texts every find reads and the facts the document-wide
        // lookups read (bookmarks, controls, revisions, frames), read once as the
        // document opens: each later version reads only the blocks it changed, and
        // no edit call pays for the first reading of a long part
        for part in &opened.doc().docx().map_err(SessionError::format)?.parts {
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
            versions: Arc::new(Versions::new()),
            default_pages: pages,
        })
    }

    /// The Word document `doc` is, if it is one.
    #[must_use]
    pub fn of(doc: &dyn FormatDoc) -> Option<&Word> {
        doc.as_any().downcast_ref()
    }

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
        let now = ooxml_word_edit::validate::problems(version.doc()).map_err(SessionError::format)?;
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

    fn readings(&self) -> Arc<dyn Any + Send + Sync> {
        Arc::clone(&self.versions) as Arc<dyn Any + Send + Sync>
    }

    fn run(&self, tool: &dyn Tool, run: &FormatRun<'_>) -> ToolResult<Value> {
        let none = WordHost::default();
        let host = WordHost::of(run.context).unwrap_or(&none);
        let layout = self.layout(host);
        let layout_cancel = run.cancel.layout();
        let made = Mutex::new(None);
        let call = Call {
            doc: run.doc,
            name: run.named,
            word: Some(self),
            caller: &run.context.caller,
            cancel: run.cancel,
            date: run.date.to_owned(),
            setup: ooxml_word_edit::Setup {
                as_given: host.as_given,
            },
            locale: run.locale,
            made: &made,
            layout: layout.as_deref(),
            layout_limit: layout_cancel.as_ref().map(limit),
            raster: host
                .raster
                .as_deref()
                .or_else(|| self.default_pages.as_ref().and_then(|p| p.raster.as_deref())),
            image_budgets: &run.context.image_budgets,
            blobs: run.context.blobs.as_deref(),
            written: &host.written,
            others: run.others,
            tables: host.tables.as_deref(),
            sources: &host.sources,
            counts: host.counts,
            limits: run.context.limits,
            tracking: self.tracking(),
            placer: host.placer.as_deref(),
        };
        let mut effects = ooxml_tools::OpEffects::default();
        let mut env = CallEnv {
            cancel: run.cancel,
            format: &call,
            effects: &mut effects,
        };
        // what its edits made is the call's even where the handler panics after them
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tool.run(run.arguments, &mut env)));
        let made = made.into_inner().unwrap_or_else(PoisonError::into_inner);
        if let Some(made) = made {
            *run.made.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(made));
        }
        let mut result = ran.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        effects.give_to(&mut result);
        // An id a version removed, refused as one the document lacks: said as such.
        match &result.outcome {
            Outcome::Failed(error) => {
                let replaced = run.doc.word().map(|w| w.ids().replaced());
                let none = BTreeMap::new();
                crate::refused::removed(self.retired(), replaced.unwrap_or(&none), run.arguments, error)
                    .map_or(result, ToolResult::failed)
            }
            Outcome::Done(_) => result,
        }
    }

    fn finish(&self, made: Box<dyn Any + Send>, run: &FormatRun<'_>) -> Option<Finished> {
        let made = *made.downcast::<Made>().ok()?;
        // the ids each version retired, by whom
        for (seq, delta) in &made.ids {
            let by = RetiredMade {
                seq: seq.0,
                caller: run.context.caller.name(),
                tool: Some(run.tool),
                history: false,
            };
            self.retired().note(&by, delta);
        }
        let one = made.first.seq() == made.last.seq();
        let none = WordHost::default();
        let host = WordHost::of(run.context).unwrap_or(&none);
        // the host's layout, else the session's own engine of the document: measured only
        // where it laid out the version before (`reflow_after_edit`: a document never laid
        // out is not laid out for it), and never inside a batch (its checks measure once,
        // after its last op)
        let layout_cancel = run.cancel.layout();
        let limit = layout_cancel.as_ref().map(limit);
        let reflow = self
            .layout(host)
            .filter(|_| !made.batch)
            .and_then(|layout| crate::reflowed::after_edit(&*layout, self, &made, (limit, run.cancel)));
        let format = WordMade {
            touched: one.then(|| made.touched.iter().map(crate::version::touched_json).collect()),
            validation: made.validation,
            reflow,
        };
        Some(Finished {
            first: made.first.seq().0,
            before: Self::any(made.before),
            last: Self::any(made.last),
            batch: made.batch,
            format: Some(Arc::new(format)),
        })
    }

    fn own_source(&self, at: &AnyVersion, address: &str) -> Option<String> {
        crate::own_sources::own_source(at.snapshot::<WordDoc>()?.doc(), address)
    }

    fn source_hint(&self, at: &AnyVersion, value: &str) -> Option<String> {
        crate::own_sources::source_hint(at.snapshot::<WordDoc>()?.doc(), value)
    }

    fn checks(
        &self,
        at: &AnyVersion,
        made: Option<&(dyn Any + Send + Sync)>,
    ) -> Option<serde_json::Map<String, Value>> {
        let made = made.and_then(|m| m.downcast_ref::<WordMade>());
        let validation = match made {
            Some(made) => crate::problems::structured(&made.validation),
            None => FormatDoc::validation(self, at)?,
        };
        let mut checks = serde_json::Map::new();
        checks.insert("validate".into(), validation.to_json());
        checks.insert(
            "reflow".into(),
            made.and_then(|m| m.reflow.clone()).unwrap_or(Value::Null),
        );
        Some(checks)
    }

    fn validation(&self, at: &AnyVersion) -> Option<ooxml_session::problems::Validation> {
        let version = at.snapshot::<WordDoc>()?;
        let delta = Word::validation(self, version).ok()?;
        Some(crate::problems::structured(&delta))
    }

    fn touched(&self, before: &AnyVersion, after: &AnyVersion, made: Option<&(dyn Any + Send + Sync)>) -> Vec<Value> {
        if let Some(touched) = made
            .and_then(|m| m.downcast_ref::<WordMade>())
            .and_then(|m| m.touched.clone())
        {
            return touched;
        }
        let (Some(then), Some(now)) = (before.snapshot::<WordDoc>(), after.snapshot::<WordDoc>()) else {
            return Vec::new();
        };
        ooxml_word_edit::touched::touched(then.doc(), now.doc())
            .map(|found| found.iter().map(crate::version::touched_json).collect())
            .unwrap_or_default()
    }

    fn group_state(&self) -> Option<Arc<dyn Any + Send + Sync>> {
        Some(Arc::new(self.tracking()))
    }

    fn restore_group(&self, state: &(dyn Any + Send + Sync)) {
        if let Some(tracking) = state.downcast_ref::<Option<String>>() {
            self.restore_tracking(tracking.clone());
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Word {
    /// The layout a call reads: the host's where it gives one, else the session's own engine
    /// of the document.
    pub(crate) fn layout(&self, host: &WordHost) -> Option<Arc<dyn WordLayout>> {
        host.layout.clone().or_else(|| self.own_pages())
    }
}

/// What a Word document's changing call made beside its versions ([`ooxml_session::CallMade`]'s
/// `format`): read with [`WordMade::of`].
#[derive(Debug, Clone)]
pub struct WordMade {
    /// What the call's edit touched (`touched` entries), where it made one version (else
    /// `None`: [`WordVersion::touched_since`] the version before `first` says).
    pub touched: Option<Vec<Value>>,
    /// The validation delta of `last` against the problems the document opened with.
    pub validation: Delta,
    /// The reflow its checks report (docx-agent's `reflow_json` form, or `"stale: call
    /// check"`, or a `"timeout: …"` where laying `last` out passed the call's limit), where
    /// the call had the host's layout and the version its first edit was applied to was laid
    /// out (a check saw it, or the engine laid it out last); else `None`.
    pub reflow: Option<Value>,
}

impl WordMade {
    /// What `made`'s Word document said of it, where it is one.
    #[must_use]
    pub fn of(made: &ooxml_session::CallMade) -> Option<&WordMade> {
        made.format.as_deref()?.downcast_ref()
    }
}

/// `cancel` as a Word tool is handed it.
pub(crate) fn limit<'a>(cancel: &'a LayoutCancel<'_>) -> LayoutLimit<'a> {
    LayoutLimit {
        cancel,
        timeout: cancel.timeout(),
    }
}

/// The store's clock and the context's cache budget, as a Word document's editor reads them.
fn ports(context: &Context) -> (Arc<StoreClock>, Arc<Budget>) {
    (
        Arc::new(StoreClock(Arc::clone(&context.clock))),
        Arc::new(Budget(Arc::clone(&context.budget))),
    )
}

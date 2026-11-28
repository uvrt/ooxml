//! Owns: Word's format (design phase 3, section 3): [`WordFormat`], how a Word document
//! opens -- read by the edit core, its ids (with those it had before it was saved), the
//! problems it opened with -- with its tools; and one open Word document as the session
//! holds it ([`Word`], its [`FormatDoc`]): its versions in its editor over `ooxml-word-edit`'s
//! [`WordDoc`], in a [`Stored`] through [`WordStore`], and what the host keeps of it beside
//! its versions: the version it was opened as (what a session's own edits are told from),
//! its tracking mode (each later edit's revision policy), the ids its versions retired, the
//! rows its table calls wrote and its page engine where the session lays it
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
    AnyVersion, BatchHold, Finished, Format, FormatDoc, FormatRun, Holds, LayoutCancel, OpenEnv, PartUpdate, Registry,
    SessionError, StepBy, StoreClock, Stored, StoredDoc,
};
use ooxml_store::{Caller, Snapshot, Store};
use ooxml_tools::{CallEnv, Outcome, Tool, ToolError, ToolResult};
use ooxml_tools_word::{LayoutLimit, Made, MadeBy, RowsWritten, WordLayout};
use ooxml_word_edit::validate::Delta;
use ooxml_word_edit::{Change, EarlierIds, Editor, IdsChange, WordDoc};
use ooxml_word_model::ids::LoadedState;
use ooxml_word_pages::layout_facts::sizes::Minimum;
use ooxml_xml::Tree;
use serde_json::Value;
use std::any::Any;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// The tool that sets a document's tracking mode: a batch with it holds the mode alone.
const TRACKING_TOOL: &str = "word_set_tracking";

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

/// Word documents (`docx`), laid out by the session with `pages` where given, with the
/// application's settings of the format (docx-agent's `docx_format`).
#[derive(Debug, Clone, Default)]
pub struct WordFormat {
    /// What the session lays its Word documents out with, each document's engine its own
    /// (fonts and memory budget); `None`: only with the layout a call's host gives.
    pub pages: Option<crate::SessionPages>,
    /// The application's minimum text size (`docx_format(min_text_size=...)`, round 17 P6b;
    /// never a tool argument): one number for every kind of text, or one per kind, read with
    /// [`ooxml_tools_word::minimum_of`]. `check` reports the text drawn below it where the
    /// call gives no `min_size` of its own; `None`: no minimum.
    pub min_text_size: Option<Minimum>,
    /// The text calls write is written as given, none of the spacing its language wants
    /// (docx-agent's `docx_format(typography=False)`); `false`: spaced by its language.
    pub as_given: bool,
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
        let word = Word::open(package, env, &EarlierIds::default(), self)?;
        Ok(Arc::new(word))
    }

    fn open_with_state(
        &self,
        package: Package,
        state: &str,
        env: &OpenEnv<'_>,
    ) -> Result<ooxml_session::WithState, SessionError> {
        let (word, numbers) = Word::open_seeded(package, env, state, self).map_err(|error| {
            if crate::refuses_state(&error) {
                SessionError::StateRefused {
                    kind: ooxml_session::DocKind::Docx,
                    reason: Some(error.to_string()),
                }
            } else {
                error
            }
        })?;
        let number = |n: i64| u64::try_from(n).unwrap_or(0);
        Ok(ooxml_session::WithState {
            doc: Arc::new(word),
            numbers: Some((number(numbers.0), number(numbers.1))),
        })
    }

    /// docx-agent's `detect`: a package named as a Word file, or one with a Word document's
    /// main part.
    fn detect(&self, bytes: &[u8], name: &str) -> bool {
        if !bytes.starts_with(b"PK") {
            return false;
        }
        let lower = name.to_lowercase();
        [".docx", ".docm", ".dotx", ".dotm"].iter().any(|e| lower.ends_with(e))
            || ooxml_session::package_has_part(bytes, "word/document.xml")
    }

    fn gives_number_texts(&self) -> bool {
        true
    }

    fn gives_scale_values(&self) -> bool {
        true
    }

    fn new_document(&self, made: &ooxml_session::NewDocument<'_>, env: &OpenEnv<'_>) -> Result<Vec<u8>, ToolError> {
        crate::new_document(made, env)
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
        Change::Received {
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
    /// session's alone (no host keeps a copy). Changed only by who holds [`Word::held`]
    /// alone.
    tracking: Mutex<Option<String>>,
    /// The tracking mode held: shared by an edit for its whole call and by a host's batch for
    /// its whole run, alone while the mode changes -- so a change of it never falls between
    /// an edit's prepare and its commit, nor between two ops of a batch (design phase 3 §4).
    held: RwLock<()>,
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
    /// Its changes feed, within the session's memory budget.
    pub(crate) feed: crate::feed::Feed,
    /// What the session lays it out with by default ([`WordFormat::pages`]).
    pub(crate) default_pages: Option<crate::SessionPages>,
    /// The application's minimum text size for its checks ([`WordFormat::min_text_size`]).
    pub(crate) min_text_size: Option<Minimum>,
    /// Its calls write text as given ([`WordFormat::as_given`]).
    pub(crate) as_given: bool,
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
        format: &WordFormat,
    ) -> Result<Word, SessionError> {
        let (clock, budget) = ports(env.context);
        let editor = Editor::open_with(package, env.store, clock, budget, earlier).map_err(SessionError::format)?;
        Self::of_editor(editor, env.context, format)
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
        format: &WordFormat,
    ) -> Result<(Word, LoadedState), SessionError> {
        let (clock, budget) = ports(env.context);
        let (editor, loaded) =
            Editor::open_with_state(package, env.store, clock, budget, state).map_err(SessionError::format)?;
        Ok((Self::of_editor(editor, env.context, format)?, loaded))
    }

    /// `package` opened with its id state as [`Word::open_state`] opens it, the ids the
    /// state retired seeded as the session's record: the document, and the state's
    /// `(version, last)` for its history's numbers to go on from.
    ///
    /// # Errors
    /// As [`Word::open_state`].
    pub(crate) fn open_seeded(
        package: Package,
        env: &OpenEnv<'_>,
        state: &str,
        format: &WordFormat,
    ) -> Result<(Word, (i64, i64)), SessionError> {
        let (word, loaded) = Self::open_state(package, env, state, format)?;
        word.retired().seed(crate::session::retired_rows(&loaded));
        Ok((word, (loaded.versions.version, loaded.versions.last)))
    }

    /// What the id state of its version numbered `number` says beside the ids (docx-agent's
    /// `hooks.id_state`): that number and the highest its history gave, and each retired id
    /// with the number of the step that retired it -- one whose step is still ending left
    /// out -- and who made that step (`tool (key=value, ...)`).
    pub(crate) fn id_state_of(&self, number: u64) -> crate::IdStateOf {
        let signed = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
        let mut gone: BTreeMap<u32, i64> = BTreeMap::new();
        let mut drawings_gone: BTreeMap<u64, i64> = BTreeMap::new();
        let mut earlier: BTreeMap<i64, Option<String>> = BTreeMap::new();
        let mut said: BTreeMap<i64, String> = BTreeMap::new();
        for (id, by) in self.retired().list() {
            let then = match by {
                crate::RetiredBy::Step { seq, caller, tool } => {
                    let Some(then) = self.retired().number_of(seq) else {
                        continue;
                    };
                    let then = signed(then);
                    if let Some(tool) = tool.filter(|t| !t.is_empty()) {
                        said.insert(then, step_words(&tool, caller.as_deref()));
                    }
                    then
                }
                crate::RetiredBy::Earlier { version, said: words } => {
                    let then = signed(version);
                    earlier.insert(then, words);
                    then
                }
            };
            match id {
                ooxml_word_model::ids::HeldId::Block(value) => gone.insert(value, then),
                ooxml_word_model::ids::HeldId::Drawing(value) => drawings_gone.insert(value, then),
            };
        }
        crate::IdStateOf {
            version: signed(number),
            last: signed(number.max(self.stored.highest())),
            gone: gone.into_iter().collect(),
            drawings_gone: drawings_gone.into_iter().collect(),
            earlier: earlier.into_iter().collect(),
            said: said.into_iter().collect(),
        }
    }

    /// A Word document's state as the session holds it, from its editor.
    fn of_editor(editor: Editor, context: &Context, format: &WordFormat) -> Result<Word, SessionError> {
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
            tracking: Mutex::new(None),
            held: RwLock::new(()),
            last_check: Arc::new(ooxml_tools_word::LastCheck::with_languages(
                ooxml_word_edit::facts::Detections::new(Arc::new(Budget(Arc::clone(&context.budget)))),
            )),
            images: ooxml_tools_word::PageImages::new(Some(Arc::clone(&context.budget))),
            rows_written: RowsWritten::default(),
            made_by: MadeBy::default(),
            pages: crate::pages::DocPages::default(),
            views: crate::live_view::LiveViews::new(Arc::clone(&context.budget)),
            feed: crate::feed::Feed::new(Arc::clone(&context.budget)),
            default_pages: format.pages.clone(),
            min_text_size: format.min_text_size.clone(),
            as_given: format.as_given,
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

    /// The author edits are tracked as now, if any. (Read it under [`Word::tracking_held`]
    /// for it to stay so until the edit commits.)
    pub(crate) fn tracking(&self) -> Option<String> {
        self.tracking.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The tracking mode held shared, for an edit's whole call or a host's batch's whole run:
    /// `word_set_tracking` waits until those holding it have committed. (A panic while it is
    /// held leaves the plain value whole: the poison is ignored.)
    pub(crate) fn tracking_held(&self) -> RwLockReadGuard<'_, ()> {
        self.held.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// The tracking mode held alone: `word_set_tracking`'s whole call, or a host's batch
    /// that sets it.
    pub(crate) fn tracking_alone(&self) -> RwLockWriteGuard<'_, ()> {
        self.held.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Set the tracking author (`None`: untracked); the caller holds the mode alone
    /// ([`Word::tracking_alone`], or a batch's hold of it alone).
    pub(crate) fn set_tracking(&self, author: Option<String>) {
        *self.tracking.lock().unwrap_or_else(PoisonError::into_inner) = author;
    }

    /// A version of it, erased.
    pub(crate) fn any(version: Snapshot<WordDoc>) -> AnyVersion {
        Stored::<WordDoc, WordStore>::any(version)
    }

    /// The head.
    pub(crate) fn head(&self) -> Snapshot<WordDoc> {
        self.stored.head()
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

    fn changes(&self, ask: &ooxml_session::FeedAsk<'_>) -> Option<Result<ooxml_session::FeedAnswer, ToolError>> {
        Some(crate::feed::hosted(self, ask))
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
    ) -> Result<(AnyVersion, bool, u32), SessionError> {
        let (version, changed, attempts) = self.stored.update(caller, tool, update)?;
        Ok((Self::any(version), changed, attempts))
    }

    fn run(&self, tool: &dyn Tool, run: &FormatRun<'_>) -> ToolResult<Value> {
        // a strict Open XML document is only laid out and painted (ooxml_tools_word::strict)
        let strict =
            self.opened.doc().strict() || run.others.iter().any(|(_, o)| o.word().is_some_and(WordDoc::strict));
        if strict && !ooxml_tools_word::strict::RENDERED.contains(&run.tool) {
            return ToolResult::failed(ooxml_tools_word::strict::refused());
        }
        let none = WordHost::default();
        let host = WordHost::of(run.context).unwrap_or(&none);
        let layout = self.layout(host);
        let layout_cancel = run.cancel.layout();
        let made = Mutex::new(None);
        // an op of a host's batch keeps what it would keep of the document for its attempt
        let kept = run.txn.map(|_| crate::call::Kept {
            rows_written: self.rows_written.attempt(),
            made_by: self.made_by.attempt(),
        });
        let call = Call {
            doc: run.doc,
            name: run.named,
            word: Some(self),
            caller: &run.context.caller,
            cancel: run.cancel,
            date: run.date.to_owned(),
            setup: ooxml_word_edit::Setup {
                as_given: host.as_given || self.as_given,
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
            written: &run.context.values.written,
            others: run.others,
            tables: host.tables.as_deref(),
            sources: &run.context.values.sources,
            one_values: &run.context.values.one_values,
            held_tables: &run.context.values.tables,
            counts: host.counts,
            limits: run.context.limits,
            tracking: self.tracking(),
            placer: host.placer.as_deref(),
            txn: run.txn,
            kept: kept.as_ref(),
            deferred: run.deferred,
        };
        // a check in a batch's transaction lays out between the batch's changes, unless the
        // version it reads is the one laid out last
        let between = crate::batch_said::check_between(run, layout.as_deref());
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
        if let (Some(said), Outcome::Done(_)) = (between, &result.outcome) {
            result.warnings.push(said.to_owned());
        }
        if let Some(kept) = kept {
            Self::defer(run, kept, &result);
        }
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
        // the ids each version retired, by whom; an op of a host's batch's, once the batch is
        // published, by the version published
        if run.txn.is_some() {
            let ids: Vec<_> = made.ids.iter().map(|(_, delta)| delta.clone()).collect();
            let (caller, tool) = (run.context.caller.name().map(str::to_owned), run.tool.to_owned());
            let keep: ooxml_session::Deferred = Box::new(move |doc, at, _, _| {
                if let Some(word) = Word::of(doc) {
                    for delta in &ids {
                        let by = RetiredMade {
                            seq: at.seq(),
                            caller: caller.as_deref(),
                            tool: Some(&tool),
                            history: false,
                        };
                        word.retired().note(&by, delta);
                    }
                }
                Vec::new()
            });
            run.deferred.lock().unwrap_or_else(PoisonError::into_inner).push(keep);
        } else {
            for (seq, delta) in &made.ids {
                let by = RetiredMade {
                    seq: seq.0,
                    caller: run.context.caller.name(),
                    tool: Some(run.tool),
                    history: false,
                };
                self.retired().note(&by, delta);
            }
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
            lock: ooxml_session::LockSpent {
                held: made.held,
                waited: made.waited,
            },
        })
    }

    fn own_source(&self, at: &AnyVersion, address: &str) -> Option<String> {
        crate::own_sources::own_source(at.snapshot::<WordDoc>()?.doc(), address)
    }

    fn source_hint(&self, at: &AnyVersion, value: &str) -> Option<String> {
        crate::own_sources::source_hint(at.snapshot::<WordDoc>()?.doc(), value)
    }

    fn number_texts(
        &self,
        at: &AnyVersion,
        read: &ooxml_session::ValueRead<'_>,
    ) -> Option<Result<ooxml_session::ReadTexts, ToolError>> {
        let version = at.snapshot::<WordDoc>()?;
        Some(crate::values::number_texts(self, version, read))
    }

    fn scale_values(
        &self,
        at: &AnyVersion,
        scale: &ooxml_session::scales::Scale,
        read: &ooxml_session::ValueRead<'_>,
    ) -> Option<Result<ooxml_session::ReadValues, ToolError>> {
        let version = at.snapshot::<WordDoc>()?;
        Some(crate::values::scale_values(self, version, scale, read))
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

    fn batch_checks(
        &self,
        before: Option<&AnyVersion>,
        after: &AnyVersion,
        context: &ooxml_session::CallContext,
        cancel: &ooxml_session::CallCancel<'_>,
    ) -> Option<serde_json::Map<String, Value>> {
        let mut checks = self.checks(after, None)?;
        let reflow = before
            .and_then(|b| b.snapshot::<WordDoc>())
            .zip(after.snapshot::<WordDoc>())
            .and_then(|(start, head)| self.reflow_between(start, head, context, cancel));
        checks.insert("reflow".into(), reflow.unwrap_or(Value::Null));
        Some(checks)
    }

    fn validation(&self, at: &AnyVersion) -> Option<ooxml_session::problems::Validation> {
        let version = at.snapshot::<WordDoc>()?;
        let delta = Word::validation(self, version).ok()?;
        Some(crate::problems::structured(&delta))
    }

    fn baseline(&self) -> Vec<ooxml_session::problems::Problem> {
        self.editor().baseline().iter().map(crate::problems::problem).collect()
    }

    fn summary(&self, at: &AnyVersion) -> Option<serde_json::Map<String, Value>> {
        let version = at.snapshot::<WordDoc>()?;
        match crate::host_reads::summary_of(version.doc()) {
            Ok(Value::Object(summary)) => Some(summary),
            _ => None,
        }
    }

    fn state(&self, at: &AnyVersion) -> Option<String> {
        let version = at.snapshot::<WordDoc>()?;
        crate::host_reads::id_state_of(version.doc(), &self.id_state_of(at.number())).ok()
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

    fn hold_for_batch(&self, tools: &[&str]) -> Option<BatchHold<'_>> {
        // a batch that sets the mode holds it alone, and its new mode is kept before it lets
        // go (`Call::edit_tracking_in`); any other holds it shared
        Some(if tools.contains(&TRACKING_TOOL) {
            Box::new(self.tracking_alone())
        } else {
            Box::new(self.tracking_held())
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Word {
    /// What changed between `start` and `head` reflowed, measured once (a batch's, after its
    /// last op): laid out by the call's layout, else the document's own engine -- only where
    /// that engine laid out `start` (`reflow_after_edit`); `None` where nothing is measured.
    pub(crate) fn reflow_between(
        &self,
        start: &Snapshot<WordDoc>,
        head: &Snapshot<WordDoc>,
        context: &ooxml_session::CallContext,
        cancel: &ooxml_session::CallCancel<'_>,
    ) -> Option<Value> {
        if head.seq() == start.seq() {
            return None;
        }
        let none = WordHost::default();
        let layout = self.layout(WordHost::of(context).unwrap_or(&none))?;
        let layout_cancel = cancel.layout();
        let limit = layout_cancel.as_ref().map(limit);
        let reading = start.doc().docx().ok()?;
        let from = ooxml_tools_word::EditedFrom {
            version: start.seq().0,
            reading: &reading,
        };
        ooxml_tools_word::reflow_after_edit(
            &*layout,
            &self.last_check,
            from,
            &head.doc().docx().ok()?,
            head.doc().ids().aliases(),
            (limit, cancel),
        )
    }

    /// The layout a call reads: the host's where it gives one, else the session's own engine
    /// of the document.
    /// What `run`, an op of a host's batch that returned `result`, keeps of the document
    /// once the batch is published (`kept`: the rows it wrote, who made what), and what it
    /// says then.
    fn defer(run: &FormatRun<'_>, kept: crate::call::Kept, result: &ToolResult<Value>) {
        let keep: ooxml_session::Deferred = Box::new(move |doc, _, _, _| {
            if let Some(word) = Word::of(doc) {
                word.rows_written.note(kept.rows_written.ids());
                word.made_by.apply(&kept.made_by);
            }
            Vec::new()
        });
        let mut deferred = run.deferred.lock().unwrap_or_else(PoisonError::into_inner);
        deferred.push(keep);
        deferred.extend(crate::batch_said::taller_later(run, result));
    }

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
pub(crate) fn ports(context: &Context) -> (Arc<StoreClock>, Arc<Budget>) {
    (
        Arc::new(StoreClock(Arc::clone(&context.clock))),
        Arc::new(Budget(Arc::clone(&context.budget))),
    )
}

/// Who made a step, in an id state's words: the tool, and the caller's context after it
/// (`word_delete (agent=a)`).
fn step_words(tool: &str, caller: Option<&str>) -> String {
    let context: Vec<String> = caller
        .and_then(|c| serde_json::from_str::<serde_json::Map<String, Value>>(c).ok())
        .map(|map| {
            map.iter()
                .map(|(key, value)| format!("{key}={}", ooxml_tools::pyfmt::str_of(value)))
                .collect()
        })
        .unwrap_or_default();
    if context.is_empty() {
        tool.to_owned()
    } else {
        format!("{tool} ({})", context.join(", "))
    }
}

//! Owns: [`Call`], what the session hands a tool it dispatches -- the document's version the
//! call reads, who calls, the call's cancellation and, for a Word document, the session's
//! editing state and the host's context (its layout, blobs, values written, the other
//! documents the call names) -- and the [`Editing`] an editing tool is run with, made from it;
//! it is a Word tool's [`FormatCall`]: it runs a Word handler ([`Run`] of
//! [`ooxml_tools_word::Word`]) on the version the call reads or through the editor.
//!
//! A new kind of host context is a field here and a `with_*` setter in [`Call::edit`],
//! not a change to the registry or to any tool.
//!
//! Must not: run a tool or know any tool's logic.

use crate::word::Word;
use crate::WordVersion;
use ooxml_base::Cancel;
use ooxml_session::{CallContext, Deferred, Opened};
use ooxml_store::{Caller, OpTxn};
use ooxml_tools::{Failure, FormatCall, Run, ToolError, ToolResult};
use ooxml_tools_word::{CallEditor, Editing, Made, Others, Version};
use serde_json::{Map, Value};
use std::any::Any;
use std::sync::{Arc, Mutex, PoisonError};

/// What a Word document's host hands a call beside the rest of its context
/// ([`CallContext::host`]): the host's layout and where blocks are laid out, its raster,
/// the tables it holds itself, the counts of its layout, and how the call's text is
/// written (the values it resolved for the call are [`CallContext::values`]).
#[derive(Clone, Default)]
pub struct WordHost {
    /// Where the call may ask where blocks are laid out: the host's layout of the version
    /// it reads (`word_inspect`'s `layout`).
    pub placer: Option<Arc<dyn ooxml_tools_word::Placer>>,
    /// The host's layout of the documents' versions, which an edit whose results come from
    /// the pages reads (`word_fields`), and `check` reads the pages of; `None`: the
    /// session's own page engine for the document, where it has one; else a call that needs
    /// the pages is refused as not supported, with the way out.
    pub layout: Option<Arc<dyn ooxml_tools_word::WordLayout>>,
    /// The faces the host draws pages with, which `render` draws a document's pages from;
    /// `None`: `render` is refused as not supported (`render: no layout`), with the way out.
    pub raster: Option<Arc<ooxml_tools_word::PageRaster>>,
    /// The tables of the open documents the host holds itself (a workbook's `t:stock`), as a
    /// `from` names them; `None`: none.
    pub tables: Option<Arc<dyn ooxml_tools_word::HostTables>>,
    /// The pages and lines of the host's layout of the version the call reads, where it
    /// has laid it out (`save_document`'s `app.xml` statistics); `None`: not laid out.
    pub counts: Option<ooxml_tools_word::LayoutCounts>,
    /// The text the call writes is written as given, none of the spacing its language wants
    /// (docx-agent's `docx_format(typography=False)`); `false`: spaced by its language.
    pub as_given: bool,
}

impl WordHost {
    /// What `context`'s host handed a Word call, if anything.
    #[must_use]
    pub fn of(context: &CallContext) -> Option<&WordHost> {
        context.host.as_deref()?.as_any().downcast_ref()
    }

    /// `self` as a call's context carries it.
    #[must_use]
    pub fn into_host(self) -> Option<Arc<dyn ooxml_session::FormatHost>> {
        Some(Arc::new(self))
    }
}

impl ooxml_session::FormatHost for WordHost {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// What a tool is handed: the document's version the call reads, who calls, the call's
/// cancellation, and for a Word document the session's editing state.
pub struct Call<'a> {
    pub(crate) doc: &'a Opened,
    /// The name the call gives its document (the host's; the session's id where the host
    /// gives none): what its reply names it by.
    pub(crate) name: &'a str,
    pub(crate) word: Option<&'a Word>,
    pub(crate) caller: &'a Caller,
    pub(crate) cancel: &'a dyn Cancel,
    /// The date edits made now carry (the host's, else the session's clock; to the second).
    pub(crate) date: String,
    /// How its edits write (text as given, or spaced by its language: the host's setting).
    pub(crate) setup: ooxml_word_edit::Setup,
    /// The locale it reads under: the call's, else the session's where the host set one;
    /// `None`: each document's own language.
    pub(crate) locale: Option<&'a ooxml_numfmt::Locale>,
    /// The versions the call's edits made (an editing tool's: [`CallEditor::made`]).
    pub(crate) made: &'a Mutex<Option<Made>>,
    /// The host's layout of the document's versions.
    pub(crate) layout: Option<&'a dyn ooxml_tools_word::WordLayout>,
    /// How long laying a version out may take in the call.
    pub(crate) layout_limit: Option<ooxml_tools_word::LayoutLimit<'a>>,
    /// The host's raster, where it hands one (`render`).
    pub(crate) raster: Option<&'a ooxml_tools_word::PageRaster>,
    /// The host's budgets of images, as they stand when the call starts.
    pub(crate) image_budgets: &'a [ooxml_tools::ImageBudget],
    /// The session's blobs, where the host hands them over.
    pub(crate) blobs: Option<&'a dyn ooxml_tools_word::Blobs>,
    /// The values the host wrote into the call's arguments for references.
    pub(crate) written: &'a [ooxml_tools_word::Written],
    /// The other documents the call names (`ooxml_tools::documents`), by id, each at the
    /// version pinned for the call when it was dispatched.
    pub(crate) others: &'a [(String, Opened)],
    /// The tables of the open documents the host holds itself.
    pub(crate) tables: Option<&'a dyn ooxml_tools_word::HostTables>,
    /// The rows of the sources the call names, as the host resolved them.
    pub(crate) sources: &'a ooxml_tools_word::Sources,
    /// Where each source naming one value finds it, as the host read it.
    pub(crate) one_values: &'a [ooxml_tools::written::OneValue],
    /// The tables of the other open documents the host read for the call.
    pub(crate) held_tables: &'a [ooxml_tools::written::HeldTable],
    /// The pages and lines of the version's layout, where the host laid it out.
    pub(crate) counts: Option<ooxml_tools_word::LayoutCounts>,
    /// The host's limits on the call's reply.
    pub(crate) limits: ooxml_tools::Limits,
    /// Who the document's edits are tracked as when the call started (what a read says of
    /// it; an edit reads the mode for its whole call instead: [`Call::edit`]).
    pub(crate) tracking: Option<String>,
    /// Where the call may ask where blocks are laid out (the host's).
    pub(crate) placer: Option<&'a dyn ooxml_tools_word::Placer>,
    /// The batch's transaction the call's edits run in, where it is an op of a host's batch
    /// of one document: what it would keep of the document waits for the batch's publish.
    pub(crate) txn: Option<&'a dyn OpTxn>,
    /// What the call keeps for its batch's attempt, where it runs in one ([`Kept`]).
    pub(crate) kept: Option<&'a Kept>,
    /// What it keeps once its batch is published.
    pub(crate) deferred: &'a Mutex<Vec<Deferred>>,
}

/// What an op of a host's batch keeps of its document for the batch's attempt, applied to the
/// document's once the batch is published: the rows its table calls wrote, the calls that
/// left tables ragged ([`ooxml_tools_word::RowsWritten::attempt`],
/// [`ooxml_tools_word::MadeBy::attempt`]).
pub(crate) struct Kept {
    pub(crate) rows_written: ooxml_tools_word::RowsWritten,
    pub(crate) made_by: ooxml_tools_word::MadeBy,
}

/// What the ops of one attempt of a host's batch keep of a Word document across them
/// ([`OpTxn::attempt`]): the tracking mode an op set (`word_set_tracking`), which the ops
/// after it write under and the document takes once the batch is published.
struct Attempt {
    tracking: Option<Option<String>>,
}

impl Call<'_> {
    /// The document at the version the call reads.
    #[must_use]
    pub fn doc(&self) -> &Opened {
        self.doc
    }

    /// The name the call gives its document, which its reply names it by.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name
    }

    /// The call's cancellation.
    #[must_use]
    pub fn cancel(&self) -> &dyn Cancel {
        self.cancel
    }

    /// The version the call reads as the session's editor holds it, with the problems the
    /// document was opened with and the version it was opened as, where the session owns
    /// it as a Word document.
    #[must_use]
    pub fn owned(&self) -> Option<ooxml_tools_word::Owned<'_>> {
        let doc = self.doc.word()?;
        let word = self.word?;
        Some(ooxml_tools_word::Owned {
            doc,
            opened: word.opened.doc(),
            baseline: word.editor().baseline(),
            last_check: &word.last_check,
            layout: self.layout,
            layout_limit: self.layout_limit,
            drawing: self.raster.map(|raster| ooxml_tools_word::Drawing {
                raster,
                images: &word.images,
                budgets: self.image_budgets,
            }),
            rows_written: Some(&word.rows_written),
            made_by: Some(&word.made_by),
            tracking: self.tracking.as_deref(),
        })
    }

    /// The other documents the call names, by id, at their pinned versions.
    #[must_use]
    pub fn others(&self) -> &[(String, Opened)] {
        self.others
    }

    /// `run` with the editing state of the Word document `word` (the call's): the session's
    /// editor, the call's caller, tracking mode and host context, and its `arguments` as
    /// taken (what the session's records keep of a call); the versions its edits
    /// made are kept for the call (its `made`), and the ids its edits drew as stand-ins are
    /// named in the reply as they were drawn where the edits landed
    /// ([`ooxml_tools_word::resolve_drawn`]); the spaces their language spacing wrote are
    /// counted in it ([`ooxml_tools_word::note_spaced`]). The tracking mode is held for the
    /// whole call ([`Word::tracking_held`]): a `word_set_tracking` waits until the call has
    /// committed, so its edits are written under the mode they commit under. An op of a
    /// host's batch reads it as the batch holds it (`FormatDoc::hold_for_batch`), or as an
    /// op before it in the attempt set it.
    pub(crate) fn edit(
        &self,
        word: &Word,
        arguments: Option<&serde_json::Map<String, Value>>,
        run: impl FnOnce(&Editing<'_>) -> ToolResult<Value>,
    ) -> ToolResult<Value> {
        if self.txn.is_some() {
            let mode = self.attempt_tracking().unwrap_or_else(|| word.tracking());
            return self.edit_under(word, mode.as_deref(), arguments, run);
        }
        let _held = word.tracking_held();
        self.edit_under(word, word.tracking().as_deref(), arguments, run)
    }

    /// The tracking mode an op of the call's batch attempt set before it, where one did.
    fn attempt_tracking(&self) -> Option<Option<String>> {
        let txn = self.txn?;
        let attempt = txn.attempt().lock().unwrap_or_else(PoisonError::into_inner);
        attempt.as_ref()?.downcast_ref::<Attempt>()?.tracking.clone()
    }

    /// [`Call::edit`] of a tool that also sets the tracking mode (`word_set_tracking`): the
    /// mode held alone for the whole call (no other edit of the document runs meanwhile),
    /// and the mode `run` leaves kept as it ends.
    pub(crate) fn edit_tracking(
        &self,
        word: &Word,
        arguments: Option<&serde_json::Map<String, Value>>,
        run: impl FnOnce(&Editing<'_>) -> (ToolResult<Value>, Option<ooxml_tools_word::Tracked>),
    ) -> ToolResult<Value> {
        if let Some(txn) = self.txn {
            return self.edit_tracking_in(word, txn, arguments, run);
        }
        let _alone = word.tracking_alone();
        let mut left = None;
        let result = self.edit_under(word, word.tracking().as_deref(), arguments, |editing| {
            let (result, set) = run(editing);
            left = set;
            result
        });
        if let Some(set) = left {
            word.set_tracking(match set {
                ooxml_tools_word::Tracked::On(author) => Some(author),
                ooxml_tools_word::Tracked::Off => None,
            });
        }
        result
    }

    /// [`Call::edit_tracking`] of an op of a host's batch: the mode `run` leaves kept for the
    /// attempt's later ops, and taken by the document once the batch is published, while the
    /// batch still holds the mode alone (`FormatDoc::hold_for_batch`).
    fn edit_tracking_in(
        &self,
        word: &Word,
        txn: &dyn OpTxn,
        arguments: Option<&serde_json::Map<String, Value>>,
        run: impl FnOnce(&Editing<'_>) -> (ToolResult<Value>, Option<ooxml_tools_word::Tracked>),
    ) -> ToolResult<Value> {
        let mode = self.attempt_tracking().unwrap_or_else(|| word.tracking());
        let mut left = None;
        let result = self.edit_under(word, mode.as_deref(), arguments, |editing| {
            let (result, set) = run(editing);
            left = set;
            result
        });
        if let Some(set) = left {
            let author = match set {
                ooxml_tools_word::Tracked::On(author) => Some(author),
                ooxml_tools_word::Tracked::Off => None,
            };
            *txn.attempt().lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(Attempt {
                tracking: Some(author.clone()),
            }));
            let keep: Deferred = Box::new(move |doc, _, _, _| {
                if let Some(word) = Word::of(doc) {
                    // (the batch holds the mode alone until its effects are kept)
                    word.set_tracking(author);
                }
                Vec::new()
            });
            self.deferred.lock().unwrap_or_else(PoisonError::into_inner).push(keep);
        }
        result
    }

    /// [`Call::edit`] with edits tracked as `author` (`None`: untracked).
    fn edit_under(
        &self,
        word: &Word,
        author: Option<&str>,
        arguments: Option<&serde_json::Map<String, Value>>,
        run: impl FnOnce(&Editing<'_>) -> ToolResult<Value>,
    ) -> ToolResult<Value> {
        let tracking = author.map(|author| ooxml_word_edit::Tracking {
            author: author.to_owned(),
            date: self.date.clone(),
        });
        let tracking = tracking.as_ref();
        // in this thread's batch on the document its edits run in the batch's transaction
        let editor = CallEditor::new(word.editor())
            .with_setup(self.setup)
            .with_owner(word.batch_owner())
            .in_txn(self.txn);
        let (rows_written, made_by) = match self.kept {
            Some(kept) => (&kept.rows_written, &kept.made_by),
            None => (&word.rows_written, &word.made_by),
        };
        let others = self.other_versions();
        let editing = Editing::new(
            &editor,
            self.caller,
            word.opened.doc(),
            self.name,
            self.cancel,
            &self.date,
        )
        .with_tracking(tracking)
        .with_layout(self.layout.map(|layout| layout as &dyn ooxml_word_edit::FieldLayout))
        .with_layout_limit(self.layout_limit)
        .with_others(
            Others::new(&others)
                .with_tables(self.tables)
                .with_sources(self.sources)
                .with_one_values(self.one_values)
                .with_held_tables(self.held_tables),
        )
        .with_blobs(self.blobs)
        .with_written(self.written)
        .with_rows_written(Some(rows_written))
        .with_made_by(Some(made_by))
        .with_arguments(arguments)
        .with_limits(self.limits)
        .with_locale(self.locale)
        .with_last_check(Some(&*word.last_check));
        // what its edits made is the call's even where the handler panics after them: the
        // panic goes on to the session's edge (crate::panics), which says they stay
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&editing)));
        let made = editor.made();
        let mut result = match ran {
            Ok(result) => result,
            Err(panic) => {
                *self.made.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = made;
                std::panic::resume_unwind(panic)
            }
        };
        ooxml_tools_word::note_spaced(&mut result, made.as_ref().map_or(0, |m| m.spaced));
        *self.made.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = made;
        ooxml_tools_word::resolve_drawn(&mut result, &editor.moved());
        result
    }

    /// The version the call reads as a reading tool reads it: with the other documents it
    /// names and the session's own reading of the version.
    pub(crate) fn read<R>(&self, run: impl FnOnce(&Version<'_>) -> R) -> R {
        let others = self.other_versions();
        let version = self
            .doc
            .version_of()
            .with_placer(self.placer)
            .with_others(
                Others::new(&others)
                    .with_tables(self.tables)
                    .with_sources(self.sources)
                    .with_one_values(self.one_values)
                    .with_held_tables(self.held_tables),
            )
            .with_owned(self.owned())
            .with_limits(self.limits)
            .with_counts(self.counts);
        run(&version)
    }

    /// [`Call::others`] as a tool reads them.
    fn other_versions(&self) -> Vec<(&str, Version<'_>)> {
        self.others
            .iter()
            .map(|(id, o)| (id.as_str(), o.version_of()))
            .collect()
    }
}

impl FormatCall for Call<'_> {
    fn run(&self, tool: &str, arguments: &Map<String, Value>, run: &dyn Any) -> ToolResult<Value> {
        let Some(run) = run.downcast_ref::<Run<ooxml_tools_word::Word>>() else {
            return internal(format!("{tool} is not a Word tool"));
        };
        let word = || {
            self.word
                .ok_or_else(|| format!("{tool} edits Word documents the session owns"))
        };
        match run {
            Run::Read(read) => match self.read(|version| read(version, self.cancel)) {
                Ok(done) => ToolResult::done(done),
                Err(error) => ToolResult::failed(error),
            },
            Run::Edit(edit) => match word() {
                Ok(word) => self.edit(word, Some(arguments), |editing| edit(editing)),
                Err(message) => internal(message),
            },
            Run::Tracking(edit) => match word() {
                Ok(word) => self.edit_tracking(word, Some(arguments), |editing| edit(editing)),
                Err(message) => internal(message),
            },
        }
    }
}

/// A call no Word tool can answer here: a fault in the wiring, never the caller's.
fn internal(message: String) -> ToolResult<Value> {
    ToolResult::failed(ToolError::failure(Failure::Internal, message))
}

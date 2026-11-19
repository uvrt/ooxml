//! Owns: [`Editor`], one Word document's versions in an `ooxml-store` store with the
//! problems it opened with, and the run of a command through it: validate and prepare on a
//! snapshot, commit (rebased past concurrent commits, prepared again on a conflict), then
//! the result from the change and the version after it ([`Edited`]): what it touched and
//! the validation delta (design section 5, "Results").
//!
//! Must not: hold anything but the store and the baseline; read the clock but through the
//! store's port; run a command's effects outside its transaction.

use crate::doc::WordDoc;
use crate::touched::{touched, Touched};
use crate::validate::{problems, Delta, Problem};
use crate::{EditError, Result};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, Command, CommitTiming, Snapshot, Store, StoreConfig};
use std::sync::Arc;
use std::time::Duration;

/// One Word document's versions, with the problems it opened with.
pub struct Editor {
    store: Store<WordDoc>,
    baseline: Vec<Problem>,
    clock: Arc<dyn Clock>,
}

/// Where a run's time went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EditTiming {
    /// Validate, prepare and commit (the store's measure, redo and waiting included).
    pub commit: CommitTiming,
    /// Working out what the change touched.
    pub touched: Duration,
    /// The validation delta.
    pub validation: Duration,
}

/// A command's result after its commit.
#[derive(Debug)]
pub struct Edited<O> {
    /// What the command returned.
    pub out: O,
    /// The version made (the one it ran on when it changed nothing).
    pub version: Snapshot<WordDoc>,
    /// The version it was applied to (the head it was published after); `None` when it
    /// changed nothing.
    pub previous: Option<Snapshot<WordDoc>>,
    /// Whether it changed the document.
    pub changed: bool,
    /// What it touched, per part.
    pub touched: Vec<Touched>,
    /// Its version's problems against those the document opened with.
    pub validation: Delta,
    /// How many times it was prepared.
    pub attempts: u32,
    /// Whether it ran under the commit lock after conflicts.
    pub exclusive: bool,
    /// Where its time went.
    pub timing: EditTiming,
}

impl Editor {
    /// The document whose first version is `package`, its caches within `budget`.
    ///
    /// # Errors
    /// When the package is not a Word document a reading can open.
    pub fn open(
        package: Package,
        config: StoreConfig,
        clock: Arc<dyn Clock>,
        budget: Arc<dyn CacheBudget>,
    ) -> Result<Editor> {
        let doc = WordDoc::open(package, budget)?;
        let baseline = problems(&doc)?;
        Ok(Editor {
            store: Store::new(doc, config, Arc::clone(&clock)),
            baseline,
            clock,
        })
    }

    /// The store.
    #[must_use]
    pub fn store(&self) -> &Store<WordDoc> {
        &self.store
    }

    /// The problems the document opened with.
    #[must_use]
    pub fn baseline(&self) -> &[Problem] {
        &self.baseline
    }

    /// Run `command` as `caller`: commit it, then report what it touched and the
    /// validation delta of the version it made.
    ///
    /// # Errors
    /// The command's refusal; nothing changed.
    pub fn run<C>(&self, caller: &Caller, command: &C) -> Result<Edited<C::Out>>
    where
        C: Command<WordDoc, Error = EditError>,
    {
        let committed = self.store.run(caller, command)?;
        let started = self.clock.monotonic();
        let touched = match &committed.previous {
            Some(previous) if committed.changed => touched(previous.doc(), committed.version.doc())?,
            _ => Vec::new(),
        };
        let found = self.clock.monotonic();
        let validation = Delta::of(&self.baseline, &problems(committed.version.doc())?);
        let validated = self.clock.monotonic();
        Ok(Edited {
            out: committed.out,
            changed: committed.changed,
            version: committed.version,
            previous: committed.previous,
            touched,
            validation,
            attempts: committed.attempts,
            exclusive: committed.exclusive,
            timing: EditTiming {
                commit: committed.timing,
                touched: found.saturating_sub(started),
                validation: validated.saturating_sub(found),
            },
        })
    }
}

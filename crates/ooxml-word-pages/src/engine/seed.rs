//! Owns: what a page engine is made from and what it laid out last, readable while another
//! caller holds the engine ([`EngineSeed`]): a caller that will not wait for a busy engine
//! learns from it whether a version was laid out there, and lays versions out on a spare
//! engine of the same kind (the same fonts, options and budgets; nothing laid out); and
//! [`lock_engine`], an engine locked, rebuilt from its seed where a panic left it poisoned.
//!
//! Must not: be written but by its engine, or hold more than the engine's last version.

use super::{EngineBudgets, EngineOptions, PageEngine};
use crate::fonts::FontSetup;
use crate::version::LaidVersion;
use ooxml_opc::Package;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The last version an engine laid out, and the package it was laid out from.
type Published = Option<(Package, Arc<LaidVersion>)>;

/// What a page engine is made from, and what it laid out last ([`PageEngine::seed`]).
#[derive(Clone)]
pub struct EngineSeed {
    setup: Arc<FontSetup>,
    options: EngineOptions,
    budgets: EngineBudgets,
    last: Arc<Mutex<Published>>,
}

impl std::fmt::Debug for EngineSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineSeed").finish_non_exhaustive()
    }
}

impl EngineSeed {
    pub(super) fn new(setup: Arc<FontSetup>, options: EngineOptions, budgets: EngineBudgets) -> EngineSeed {
        EngineSeed {
            setup,
            options,
            budgets,
            last: Arc::new(Mutex::new(None)),
        }
    }

    /// The last version its engine laid out, where it was laid out from `package`'s parts
    /// ([`PageEngine::laid_last`]); else `None`. Never waits for the engine.
    #[must_use]
    pub fn laid_last(&self, package: &Package) -> Option<Arc<LaidVersion>> {
        let last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        let (from, version) = last.as_ref()?;
        from.same_parts(package).then(|| Arc::clone(version))
    }

    /// A spare engine of the same kind: its fonts, options and budgets, nothing laid out
    /// (its own memos, its own seed).
    #[must_use]
    pub fn spare(&self) -> PageEngine {
        PageEngine::new(Arc::clone(&self.setup), self.options.clone(), self.budgets.clone())
    }

    /// What its engine laid out last now.
    pub(super) fn publish(&self, last: Published) {
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = last;
    }

    /// Its engine made again: the same fonts, options and budgets and this seed, nothing
    /// laid out (its last version let go).
    fn rebuilt(&self) -> PageEngine {
        let mut engine = PageEngine::new(Arc::clone(&self.setup), self.options.clone(), self.budgets.clone());
        engine.seed = self.clone();
        self.publish(None);
        engine
    }
}

/// `engine` locked. Where a panic while it was held left it poisoned, the engine -- whose
/// memos and versions may be half made -- is not reused: it is made again from its seed
/// (nothing laid out; the next version is laid out whole) and the lock is cleared.
pub fn lock_engine(engine: &Mutex<PageEngine>) -> MutexGuard<'_, PageEngine> {
    match engine.lock() {
        Ok(held) => held,
        Err(poisoned) => {
            let mut held = poisoned.into_inner();
            let seed = held.seed.clone();
            *held = seed.rebuilt();
            engine.clear_poison();
            held
        }
    }
}

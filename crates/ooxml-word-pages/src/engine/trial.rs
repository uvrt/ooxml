//! Owns: a trial layout ([`PageEngine::trial`]): a package that differs from a version the
//! engine laid out by a change a check only asks about (docx-agent's `keep_trials`: a
//! range's keep set or released), laid out from that version -- incrementally, from the
//! page the change reaches until the pages line up again -- and read as facts, kept
//! nowhere: the engine's last version, its paint and what its pages said are the same
//! after as before, so the next version is laid out from the version the engine had.
//!
//! Must not: replace the engine's last version, paint, or start a generation.

use super::PageEngine;
use crate::error::PagesError;
use crate::support;
use crate::LaidVersion;
use ooxml_base::Cancel;
use ooxml_opc::Package;
use ooxml_word_layout::facts::LayoutFacts;
use std::time::Instant;

impl PageEngine {
    /// `package` laid out from `from` (a version this engine laid out), stopped where
    /// `cancel` says so: its facts, `None` where it is not laid out here -- `from` is of
    /// another generation than the engine's (its fonts changed since), `package`'s fonts
    /// are not `from`'s, or the layout falls back for it.
    ///
    /// # Errors
    /// Its Word parts do not open; [`PagesError::Stopped`] where `cancel` stopped it.
    pub fn trial(
        &mut self,
        package: &Package,
        from: &LaidVersion,
        cancel: &dyn Cancel,
    ) -> Result<Option<LayoutFacts>, PagesError> {
        let fonts = self.setup.document(package);
        let Some(generation) = self.generation.as_mut() else {
            return Ok(None);
        };
        if generation.number != from.generation || generation.fonts.epoch != fonts.epoch {
            return Ok(None);
        }
        let started = Instant::now();
        let document = generation.model.open(package)?;
        self.stats.open_time += started.elapsed();
        if generation.model.stats().context_changed || support::unsupported(&document).is_some() {
            return Ok(None);
        }
        let started = Instant::now();
        let laid = generation
            .layout
            .lay_out_until(&mut generation.model, &document, Some(&from.layout), cancel);
        self.stats.layout_time += started.elapsed();
        let Ok(laid) = laid else {
            self.stats.stopped += 1;
            return Err(PagesError::Stopped);
        };
        if support::stopped(&laid).is_some() {
            return Ok(None);
        }
        self.stats.trials += 1;
        Ok(Some(LayoutFacts::after(&laid, &from.layout, from.facts())))
    }
}

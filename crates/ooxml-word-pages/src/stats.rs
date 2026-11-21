//! Owns: [`EngineStats`], what a page engine did since it was made: the versions it laid
//! out (whole, again from the one before, answered with a fallback, stopped), the parts it
//! read, the trial layouts it made, the pages it painted and said, and its time opening and laying out.
//!
//! Must not: hold anything but counts and times.

use std::time::Duration;

/// What an engine did, since it was made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineStats {
    /// Versions laid out whole.
    pub full: u64,
    /// Versions laid out again from the version before.
    pub relaid: u64,
    /// Versions answered with a fallback.
    pub fallbacks: u64,
    /// Versions whose layout was stopped (cancelled, or past its caller's deadline).
    pub stopped: u64,
    /// Parts read again (their stored bytes changed), over every version.
    pub parts_read: u64,
    /// Parts taken as they were.
    pub parts_kept: u64,
    /// Pages painted.
    pub pages_painted: u64,
    /// Time spent opening versions (the model).
    pub open_time: Duration,
    /// Time spent laying out.
    pub layout_time: Duration,
    /// Memo entries swept (no version in use held them).
    pub swept: u64,
    /// Pages painted that were kept painted already.
    pub paint_hits: u64,
    /// Pages whose warnings were said again as the version before's same page said them,
    /// and pages walked for them.
    pub pages_said: (u64, u64),
    /// Versions asked again: the same parts as the last version laid out, which was
    /// answered (nothing laid out).
    pub kept: u64,
    /// Pages painted for their digests (a summary's or a comparison's), and pages a
    /// comparison took as digested before or as the same page in both versions.
    pub pages_digested: (u64, u64),
    /// Trial layouts made (a version with a change a check asks about, kept nowhere).
    pub trials: u64,
}

impl EngineStats {
    /// Count a version laid out: from the one before (`relaid`) or whole.
    pub(crate) fn count_laid(&mut self, relaid: bool) {
        if relaid {
            self.relaid += 1;
        } else {
            self.full += 1;
        }
    }
}

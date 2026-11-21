//! Owns: [`Stop`], what laying a version out asks whether to stop: the caller's
//! [`Cancel`] (a call's cancellation, or a host's limit on how long laying out may take),
//! asked once per top-level block of the body's flow and once per page, and kept once it
//! said stop, so every part of the layout that asks after sees the same answer.
//!
//! Must not: read a clock (the [`Cancel`] does), or stop anything itself: a layout that
//! was told to stop ends where it is, and its caller drops what it made.

use ooxml_base::Cancel;
use std::cell::Cell;

/// Whether laying a version out should stop: never, or when a [`Cancel`] says so.
pub struct Stop<'c> {
    cancel: Option<&'c dyn Cancel>,
    stopped: Cell<bool>,
}

impl std::fmt::Debug for Stop<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stop")
            .field("stopped", &self.stopped.get())
            .finish_non_exhaustive()
    }
}

impl Stop<'static> {
    /// A layout that is never stopped.
    #[must_use]
    pub fn never() -> Stop<'static> {
        Stop {
            cancel: None,
            stopped: Cell::new(false),
        }
    }
}

impl<'c> Stop<'c> {
    /// A layout stopped where `cancel` says so.
    #[must_use]
    pub fn on(cancel: &'c dyn Cancel) -> Stop<'c> {
        Stop {
            cancel: Some(cancel),
            stopped: Cell::new(false),
        }
    }

    /// Whether to stop now: asks the [`Cancel`] (until it said stop once).
    pub fn now(&self) -> bool {
        if self.stopped.get() {
            return true;
        }
        let stop = self.cancel.is_some_and(Cancel::is_cancelled);
        self.stopped.set(stop);
        stop
    }

    /// Whether the layout was told to stop (without asking again).
    #[must_use]
    pub fn stopped(&self) -> bool {
        self.stopped.get()
    }
}

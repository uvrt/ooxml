//! Owns: a call's context ([`CallContext`]: who calls, its cancellation token, its
//! deadline, the versions it reads, where it may ask where blocks are laid out), the token a host cancels a call with ([`CancelToken`]), the [`Cancel`] a call
//! runs under (its token, its deadline read from the session's [`Clock`], and the
//! session's own [`Cancel`]), and the store's clock read through the session's.
//!
//! Must not: read the system clock (the host gives the [`Clock`]) or keep state beyond a
//! call.

use crate::Opened;
use ooxml_base::{Cancel, Clock};
use ooxml_store::Caller;
use ooxml_tools_word::Placer;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A flag a host sets to stop the calls that hold it; they stop at their next safe point
/// and change nothing.
#[derive(Debug, Default)]
pub struct CancelToken(AtomicBool);

impl CancelToken {
    /// A token not yet cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the calls holding it to stop.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

impl Cancel for CancelToken {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// What a call runs with.
#[derive(Clone, Default)]
pub struct CallContext {
    /// Who calls (kept with the history step a changing call makes).
    pub caller: Caller,
    /// A token that stops the call.
    pub cancel: Option<Arc<CancelToken>>,
    /// How long the call may run; past it, it stops with `timeout`.
    pub deadline: Option<Duration>,
    /// The versions the call reads (each a document's, as [`crate::Session::commit`] or
    /// [`crate::Session::snapshot`] gave it): a document named here is read at this
    /// version, any other at its head. The host that owns the documents names the version
    /// its caller reads, so a reader never reads a version newer than its own.
    pub at: Vec<Opened>,
    /// Where the call may ask where blocks are laid out: the host's layout of the version
    /// it reads (`word_inspect`'s `layout`).
    pub placer: Option<Arc<dyn Placer>>,
}

impl std::fmt::Debug for CallContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallContext")
            .field("caller", &self.caller)
            .field("deadline", &self.deadline)
            .field("at", &self.at.iter().map(|o| (o.id().to_string(), o.version())).collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// The cancellation a call runs under.
pub(crate) struct CallCancel<'a> {
    token: Option<&'a CancelToken>,
    session: &'a dyn Cancel,
    clock: &'a dyn Clock,
    until: Option<Duration>,
}

impl<'a> CallCancel<'a> {
    pub(crate) fn new(context: &'a CallContext, session: &'a dyn Cancel, clock: &'a dyn Clock) -> Self {
        Self {
            token: context.cancel.as_deref(),
            session,
            clock,
            until: context.deadline.map(|d| clock.monotonic().saturating_add(d)),
        }
    }
}

impl Cancel for CallCancel<'_> {
    fn is_cancelled(&self) -> bool {
        self.token.is_some_and(Cancel::is_cancelled)
            || self.session.is_cancelled()
            || self.until.is_some_and(|until| self.clock.monotonic() >= until)
    }
}

/// The session's clock as the store reads it.
pub(crate) struct StoreClock(pub(crate) Arc<dyn Clock>);

impl ooxml_store::Clock for StoreClock {
    fn monotonic(&self) -> Duration {
        self.0.monotonic()
    }
}

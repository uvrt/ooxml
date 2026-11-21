//! Owns: a call's context ([`CallContext`]: who calls, its cancellation token, its
//! deadline, the versions it reads, where it may ask where blocks are laid out, the host's
//! names for its documents), the token a host cancels a call with ([`CancelToken`]), the [`Cancel`] a call
//! runs under (its token, its deadline read from the session's [`Clock`], and the
//! session's own [`Cancel`]), and the store's clock read through the session's.
//!
//! Must not: read the system clock (the host gives the [`Clock`]) or keep state beyond a
//! call.

use crate::{DocNames, Opened};
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
    /// The host's layout of the documents' versions, which an edit whose results come from
    /// the pages reads (`word_fields`); `None`: such a result is left to the host.
    pub layout: Option<Arc<dyn ooxml_word_edit::FieldLayout>>,
    /// The session's blobs as the host keeps them (an inserted picture's bytes); `None`: a
    /// call that reads one is left to the host.
    pub blobs: Option<Arc<dyn ooxml_tools_word::Blobs>>,
    /// The host's names for the documents the call names (`d1` in the host's own session),
    /// each the session's id of one: the call names its documents by these, and its reply
    /// names them so. Empty: by the session's ids.
    pub names: DocNames,
    /// The values the host wrote into the call's arguments for references (`{{d:...}}`,
    /// resolved against its datasets), which an editing call keeps on the paragraphs they
    /// went into; empty: none.
    pub written: Vec<ooxml_tools_word::Written>,
    /// The moment the host dates the call's writes by (a revision's and a comment's date,
    /// `2026-11-20T09:30:15Z`): the host's clock, to the second, as docx-agent dates them;
    /// `None`: the session's clock.
    pub date: Option<String>,
    /// The text the call writes is written as given, none of the spacing its language wants
    /// (docx-agent's `docx_format(typography=False)`); `false`: spaced by its language.
    pub as_given: bool,
}

impl std::fmt::Debug for CallContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallContext")
            .field("caller", &self.caller)
            .field("deadline", &self.deadline)
            .field(
                "at",
                &self
                    .at
                    .iter()
                    .map(|o| (o.id().to_string(), o.version()))
                    .collect::<Vec<_>>(),
            )
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

/// The date an edit made at `now` carries, as docx-agent's tools date it (the session's
/// clock, `utc_now`): UTC, to the second (`2026-11-18T09:30:15Z`).
pub(crate) fn revision_date(now: ooxml_base::context::Timestamp) -> String {
    let days = now.seconds.div_euclid(86_400);
    let seconds = now.seconds.rem_euclid(86_400);
    let minutes = seconds / 60;
    // days since 1970-01-01 to a civil date (H. Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        minutes / 60,
        minutes % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn revision_dates_are_utc_to_the_second() {
        let at = |seconds| super::revision_date(ooxml_base::context::Timestamp { seconds, nanos: 0 });
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
        assert_eq!(at(1_795_000_000), "2026-11-18T11:06:40Z");
        assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z");
    }
}

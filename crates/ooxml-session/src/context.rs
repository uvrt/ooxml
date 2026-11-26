//! Owns: a call's context ([`CallContext`]: who calls, its cancellation token, its
//! deadline, the versions it reads, where it may ask where blocks are laid out, the host's
//! names for its documents, the limits on its reply), the token a host cancels a call with ([`CancelToken`]), the [`Cancel`] a call
//! runs under (its token, its deadline read from the session's [`Clock`], and the
//! session's own [`Cancel`]), and the store's clock read through the session's.
//!
//! Must not: read the system clock (the host gives the [`Clock`]) or keep state beyond a
//! call.

use crate::{DocNames, Opened};
use ooxml_base::{Cancel, Clock};
use ooxml_store::Caller;
use serde_json::{Map, Value};
use std::any::Any;
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
    /// The caller's context as the host gave it, its keys in the order given: how a reply
    /// names who made a step (`agent=b`); `None`: the caller key's own fields.
    pub caller_context: Option<Arc<Map<String, Value>>>,
    /// A token that stops the call.
    pub cancel: Option<Arc<CancelToken>>,
    /// How long the call may run; past it, it stops with `timeout`.
    pub deadline: Option<Duration>,
    /// How long laying a document out may take in the call, from its start (docx-agent's
    /// `Limits.layout_timeout`); past it, a tool reading the layout says so (`check`'s
    /// `reflow.error`) and goes on. `None`: as long as the call.
    pub layout_timeout: Option<Duration>,
    /// The versions the call reads (each a document's, as [`crate::Session::snapshot`] gave
    /// it): a document named here is read at this
    /// version, any other at its head. The host that owns the documents names the version
    /// its caller reads, so a reader never reads a version newer than its own.
    pub at: Vec<Opened>,
    /// The host's budgets of images kept across calls (the session's, the round's) as they
    /// stand when the call starts, which `render` refuses past; empty: none. The call's own
    /// limit is in [`Self::limits`].
    pub image_budgets: Vec<ooxml_tools::ImageBudget>,
    /// The host's names for the documents the call names (`d1` in the host's own session),
    /// each the session's id of one: the call names its documents by these, and its reply
    /// names them so. Empty: by the session's ids.
    pub names: DocNames,
    /// The host's limits on the call's reply: a page's characters, a listing's items
    /// (docx-agent's `Limits.max_result_chars`, `Limits.max_list_items`).
    pub limits: ooxml_tools::Limits,
    /// The moment the host dates the call's writes by (a revision's and a comment's date,
    /// `2026-11-20T09:30:15Z`): the host's clock, to the second, as docx-agent dates them;
    /// `None`: the session's clock.
    pub date: Option<String>,
    /// The locale the call reads its documents under (docx-agent's document locale: a table
    /// of contents' style names and the separator between them); `None`: the session's,
    /// where the host set one ([`crate::SessionConfig::locale`]), else each document's own
    /// language (`w:themeFontLang`, then the default run language).
    pub locale: Option<ooxml_numfmt::Locale>,
    /// The session's blobs as the host keeps them (an inserted picture's bytes), for a tool
    /// that reads one; `None`: such a call is refused as not supported, with the way out.
    pub blobs: Option<Arc<dyn ooxml_tools::blobs::Blobs>>,
    /// What the host hands the call for its document's format beside the rest (a Word
    /// call's layout, blobs, values written by reference: `ooxml-session-word`'s
    /// `WordHost`), read by that format alone; `None`: nothing.
    pub host: Option<Arc<dyn FormatHost>>,
}

/// What a host hands a call for its document's format beside the rest of its context
/// ([`CallContext::host`]): a format's own type (a Word call's layout and the values the host
/// wrote, `ooxml-session-word`'s `WordHost`), which only that format reads.
pub trait FormatHost: Any + Send + Sync {
    /// Itself, for its format to read (a downcast).
    fn as_any(&self) -> &dyn Any;
}

impl std::fmt::Debug for CallContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallContext")
            .field("caller", &self.caller)
            .field("deadline", &self.deadline)
            .field("layout_timeout", &self.layout_timeout)
            .field("locale", &self.locale)
            .field("limits", &self.limits)
            .field("image_budgets", &self.image_budgets)
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
#[derive(Clone, Copy)]
pub struct CallCancel<'a> {
    token: Option<&'a CancelToken>,
    session: &'a dyn Cancel,
    clock: &'a dyn Clock,
    until: Option<Duration>,
    /// The call's limit on laying out, and when it passes.
    layout: Option<(Duration, Duration)>,
}

impl<'a> CallCancel<'a> {
    /// What `context` runs under in a session with `session`'s cancellation and `clock`.
    #[must_use]
    pub fn new(context: &'a CallContext, session: &'a dyn Cancel, clock: &'a dyn Clock) -> Self {
        let now = clock.monotonic();
        Self {
            token: context.cancel.as_deref(),
            session,
            clock,
            until: context.deadline.map(|d| now.saturating_add(d)),
            layout: context.layout_timeout.map(|t| (t, now.saturating_add(t))),
        }
    }

    /// What stops laying a document out in the call, where it has a limit on that.
    #[must_use]
    pub fn layout(&self) -> Option<LayoutCancel<'a>> {
        self.layout.map(|(timeout, until)| LayoutCancel {
            call: *self,
            timeout,
            until,
        })
    }
}

/// What stops laying a document out in a call: the call's cancellation, or its limit on
/// laying out passed.
pub struct LayoutCancel<'a> {
    call: CallCancel<'a>,
    timeout: Duration,
    until: Duration,
}

impl LayoutCancel<'_> {
    /// The call's limit on laying out, as said where it passed.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl Cancel for LayoutCancel<'_> {
    fn is_cancelled(&self) -> bool {
        self.call.is_cancelled() || self.call.clock.monotonic() >= self.until
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
pub struct StoreClock(pub Arc<dyn Clock>);

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

    /// A clock at `0.0` seconds, moved by hand.
    struct Hand(std::sync::Mutex<std::time::Duration>);

    impl ooxml_base::Clock for Hand {
        fn now(&self) -> ooxml_base::context::Timestamp {
            ooxml_base::context::Timestamp::default()
        }
        fn monotonic(&self) -> std::time::Duration {
            *self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        }
    }

    #[test]
    fn laying_out_stops_past_the_calls_limit_on_it_or_with_the_call() {
        use ooxml_base::Cancel;
        use std::time::Duration;
        let clock = Hand(std::sync::Mutex::new(Duration::from_secs(100)));
        let token = std::sync::Arc::new(super::CancelToken::new());
        let session = super::CancelToken::new();
        let context = super::CallContext {
            cancel: Some(std::sync::Arc::clone(&token)),
            layout_timeout: Some(Duration::from_secs(30)),
            ..super::CallContext::default()
        };
        let call = super::CallCancel::new(&context, &session, &clock);
        let Some(layout) = call.layout() else {
            panic!("a limit on laying out")
        };
        assert_eq!(layout.timeout(), Duration::from_secs(30));
        assert!(!layout.is_cancelled());
        *clock.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Duration::from_secs(130);
        assert!(layout.is_cancelled(), "past the limit");
        assert!(!call.is_cancelled(), "the call goes on");
        *clock.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Duration::from_secs(100);
        token.cancel();
        assert!(layout.is_cancelled(), "with the call");
        let plain = super::CallContext::default();
        assert!(super::CallCancel::new(&plain, &session, &clock).layout().is_none());
    }
}

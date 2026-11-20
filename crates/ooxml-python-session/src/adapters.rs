//! Owns: the adapters of the core's ports for a session made from Python (design section
//! 2.3: adapters live in leaf crates): the system clock, a font source with no faces (no
//! tool of this skeleton lays out), a part store that stores nothing, a cache budget
//! that grants every reservation (no cache of this skeleton reserves), and the host's blobs
//! asked of a Python object ([`PyBlobs`]).
//!
//! Must not: be reached from the core except through the ports.

use ooxml_base::context::{FaceData, FaceRequest, Timestamp};
use ooxml_base::{BaseError, CacheBudget, Cancel, Clock, Context, FontSource, Limits, Locale, PartStore};
use ooxml_tools_word::{Blob, Blobs};
use pyo3::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The system's clocks: wall time from `SystemTime`, monotonic time from an `Instant`
/// taken when the session was made.
pub(crate) struct SystemClock {
    origin: Instant,
}

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
        Timestamp {
            seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
            nanos: since.subsec_nanos(),
        }
    }

    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// No faces: a session from Python lays nothing out yet.
struct NoFonts;

impl FontSource for NoFonts {
    fn face(&self, _: &FaceRequest) -> Result<Option<FaceData>, BaseError> {
        Ok(None)
    }
}

/// Stores nothing: the host keeps the bytes a session saves.
struct NoStore;

impl PartStore for NoStore {
    fn load(&self, _: &str) -> Result<Option<Arc<[u8]>>, BaseError> {
        Ok(None)
    }

    fn store(&self, key: &str, _: Arc<[u8]>) -> Result<(), BaseError> {
        Err(BaseError::Port {
            port: "parts",
            message: format!("this session stores no blobs ({key}): the host keeps saved bytes"),
        })
    }
}

/// Grants every reservation and counts what is held.
#[derive(Default)]
struct Counted(AtomicUsize);

impl CacheBudget for Counted {
    fn reserve(&self, bytes: usize) -> bool {
        self.0.fetch_add(bytes, Ordering::Relaxed);
        true
    }

    fn release(&self, bytes: usize) {
        self.0.fetch_sub(bytes, Ordering::Relaxed);
    }

    fn used(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

/// The session's own stop: set when the host closes the session, it stops every call in
/// flight at its next safe point.
#[derive(Default)]
pub(crate) struct SessionStop(AtomicBool);

impl SessionStop {
    pub(crate) fn stop(&self) {
        self.0.store(true, Ordering::Release);
    }
}

impl Cancel for SessionStop {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// A session's context: these adapters, the default limits, the locale named.
pub(crate) fn context(stop: Arc<SessionStop>, locale: Locale) -> Context {
    Context {
        fonts: Arc::new(NoFonts),
        parts: Arc::new(NoStore),
        clock: Arc::new(SystemClock { origin: Instant::now() }),
        budget: Arc::new(Counted::default()),
        cancel: stop,
        limits: Limits::default(),
        locale,
    }
}

/// The host's blobs (an inserted picture's bytes), asked of a Python object with the
/// interpreter lock taken back: `blob(handle)` gives `(name, mime, bytes)` or `None`,
/// `listed()` every `(handle, name)` in the order they were registered. A blob's bytes are
/// copied once, when a tool asks for it; an error in Python reads as no blob.
pub(crate) struct PyBlobs(pub(crate) Py<PyAny>);

impl Blobs for PyBlobs {
    fn blob(&self, handle: &str) -> Option<Blob> {
        Python::attach(|py| {
            let found = self.0.call_method1(py, "blob", (handle,)).ok()?;
            let (name, mime, data) = found.extract::<Option<(String, String, Vec<u8>)>>(py).ok().flatten()?;
            Some(Blob {
                handle: handle.to_owned(),
                name,
                mime,
                data: Arc::from(data),
            })
        })
    }

    fn listed(&self) -> Vec<(String, String)> {
        Python::attach(|py| {
            self.0
                .call_method0(py, "listed")
                .ok()
                .and_then(|listed| listed.extract::<Vec<(String, String)>>(py).ok())
                .unwrap_or_default()
        })
    }
}

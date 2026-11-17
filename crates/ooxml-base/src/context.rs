//! Owns: the ports through which the pure core is given everything outside it (design
//! section 2.3) -- fonts, stored parts, the clock, the cache budget and cancellation -- and
//! the [`Context`] value that carries them with the limits and the locale.
//!
//! Must not: implement a port that does I/O. Adapters (file-system fonts, the Office
//! bundle, zip on disk, the system clock, a test clock) live in leaf crates; this module
//! holds only the traits and the value that carries them.

use crate::{BaseError, Limits, Locale};
use std::sync::Arc;
use std::time::Duration;

/// A font face asked for by family and style.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FaceRequest {
    /// The family name as a document names it.
    pub family: String,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
}

/// A font file's bytes and the face's index in it (for collections).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceData {
    /// The whole font file.
    pub bytes: Arc<[u8]>,
    /// The face's index in a collection; 0 for a single face.
    pub index: u32,
}

/// Where fonts come from.
pub trait FontSource: Send + Sync {
    /// The face for `request`, or `None` when the source has none.
    fn face(&self, request: &FaceRequest) -> Result<Option<FaceData>, BaseError>;
}

/// Where packages and other stored blobs are read from and written to, by key.
pub trait PartStore: Send + Sync {
    /// The blob stored under `key`, or `None`.
    fn load(&self, key: &str) -> Result<Option<Arc<[u8]>>, BaseError>;
    /// Store `bytes` under `key`.
    fn store(&self, key: &str, bytes: Arc<[u8]>) -> Result<(), BaseError>;
}

/// A point in time: seconds and nanoseconds since 1970-01-01 UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Timestamp {
    /// Whole seconds since the epoch.
    pub seconds: i64,
    /// Nanoseconds within the second.
    pub nanos: u32,
}

/// The clock: wall time for stamps (revision dates), monotonic time for deadlines.
pub trait Clock: Send + Sync {
    /// The current wall time.
    fn now(&self) -> Timestamp;
    /// Time since an arbitrary fixed origin, never going backwards.
    fn monotonic(&self) -> Duration;
}

/// A memory budget that caches reserve from and release to.
pub trait CacheBudget: Send + Sync {
    /// Reserve `bytes`; `false` when the budget cannot give them (the caller evicts or does
    /// without the cache).
    fn reserve(&self, bytes: usize) -> bool;
    /// Give back `bytes` reserved before.
    fn release(&self, bytes: usize);
    /// Bytes reserved now.
    fn used(&self) -> usize;
}

/// Cooperative cancellation, checked in every loop over blocks, pages and records.
pub trait Cancel: Send + Sync {
    /// Whether the work should stop.
    fn is_cancelled(&self) -> bool;

    /// `Err(Cancelled)` when the work should stop.
    fn check(&self) -> Result<(), BaseError> {
        if self.is_cancelled() {
            Err(BaseError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// What a session or document gives the core: its ports and its configuration values.
/// Cheap to clone (the ports are shared).
#[derive(Clone)]
pub struct Context {
    /// Fonts.
    pub fonts: Arc<dyn FontSource>,
    /// Stored blobs.
    pub parts: Arc<dyn PartStore>,
    /// The clock.
    pub clock: Arc<dyn Clock>,
    /// The cache budget.
    pub budget: Arc<dyn CacheBudget>,
    /// Cancellation.
    pub cancel: Arc<dyn Cancel>,
    /// Limits on untrusted input.
    pub limits: Limits,
    /// The locale.
    pub locale: Locale,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("limits", &self.limits)
            .field("locale", &self.locale)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct NoFonts;
    impl FontSource for NoFonts {
        fn face(&self, _: &FaceRequest) -> Result<Option<FaceData>, BaseError> {
            Ok(None)
        }
    }
    struct NoParts;
    impl PartStore for NoParts {
        fn load(&self, _: &str) -> Result<Option<Arc<[u8]>>, BaseError> {
            Ok(None)
        }
        fn store(&self, _: &str, _: Arc<[u8]>) -> Result<(), BaseError> {
            Err(BaseError::Port {
                port: "parts",
                message: "read-only".into(),
            })
        }
    }
    struct Fixed;
    impl Clock for Fixed {
        fn now(&self) -> Timestamp {
            Timestamp {
                seconds: 1_700_000_000,
                nanos: 0,
            }
        }
        fn monotonic(&self) -> Duration {
            Duration::ZERO
        }
    }
    struct Budget(AtomicUsize, usize);
    impl CacheBudget for Budget {
        fn reserve(&self, bytes: usize) -> bool {
            let now = self.0.fetch_add(bytes, Ordering::SeqCst) + bytes;
            if now > self.1 {
                self.0.fetch_sub(bytes, Ordering::SeqCst);
            }
            now <= self.1
        }
        fn release(&self, bytes: usize) {
            self.0.fetch_sub(bytes, Ordering::SeqCst);
        }
        fn used(&self) -> usize {
            self.0.load(Ordering::SeqCst)
        }
    }
    struct Flag(AtomicBool);
    impl Cancel for Flag {
        fn is_cancelled(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn a_context_carries_its_ports() {
        let cancel = Arc::new(Flag(AtomicBool::new(false)));
        let context = Context {
            fonts: Arc::new(NoFonts),
            parts: Arc::new(NoParts),
            clock: Arc::new(Fixed),
            budget: Arc::new(Budget(AtomicUsize::new(0), 100)),
            cancel: cancel.clone(),
            limits: Limits::default(),
            locale: Locale::default(),
        };
        let shared = context.clone();
        assert!(shared.budget.reserve(60));
        assert!(!context.budget.reserve(60));
        assert_eq!(context.budget.used(), 60);
        assert_eq!(context.cancel.check(), Ok(()));
        cancel.0.store(true, Ordering::SeqCst);
        assert_eq!(shared.cancel.check(), Err(BaseError::Cancelled));
        assert_eq!(context.limits.xml.max_depth, 256);
        assert!(context.parts.store("x", Arc::from(&b"y"[..])).is_err());
        assert_eq!(
            context.fonts.face(&FaceRequest {
                family: "Calibri".into(),
                bold: false,
                italic: false
            }),
            Ok(None)
        );
        assert_eq!(context.clock.now().seconds, 1_700_000_000);
    }
}

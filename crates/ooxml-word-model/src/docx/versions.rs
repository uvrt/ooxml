//! Owns: [`Versions`], one document's readings ([`Docx`]) of its latest versions, by version
//! number: a call on a version read before takes its reading as it is, and the first call
//! on a new version reads only the story parts that changed ([`Docx::open_after`]).
//!
//! Must not: hold more than its few latest versions, or anything global; the session owns
//! one per document (design section 7: caches belong to a document). Safe from any number
//! of threads: a reading is made outside the lock, which is held only to find or keep one.

use super::Docx;
use crate::WordError;
use ooxml_opc::Package;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

/// Versions kept.
const KEPT: usize = 4;

/// One document's latest readings.
#[derive(Debug, Default)]
pub struct Versions {
    kept: Mutex<VecDeque<(u64, Arc<Docx>)>>,
}

impl Versions {
    /// None yet.
    #[must_use]
    pub fn new() -> Versions {
        Versions::default()
    }

    /// The reading of version `version` of the document, whose package is `package`.
    ///
    /// # Errors
    /// As [`Docx::open`].
    pub fn docx(&self, version: u64, package: &Package) -> Result<Arc<Docx>, WordError> {
        let latest = {
            let kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((_, found)) = kept.iter().find(|(v, _)| *v == version) {
                return Ok(Arc::clone(found));
            }
            kept.iter().max_by_key(|(v, _)| *v).map(|(_, d)| Arc::clone(d))
        };
        let read = Arc::new(Docx::open_after(package, latest.as_deref())?);
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, found)) = kept.iter().find(|(v, _)| *v == version) {
            return Ok(Arc::clone(found));
        }
        kept.push_back((version, Arc::clone(&read)));
        while kept.len() > KEPT {
            let oldest = kept.iter().enumerate().min_by_key(|(_, (v, _))| *v).map(|(k, _)| k);
            if let Some(k) = oldest {
                kept.remove(k);
            }
        }
        Ok(read)
    }
}

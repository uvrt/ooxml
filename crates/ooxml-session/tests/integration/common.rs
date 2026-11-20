//! Helpers the test modules share: a context with no fonts, no stored parts, no budget limit
//! and no cancelling (one copy instead of one per module).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use ooxml_base::context::{FaceData, FaceRequest};
use ooxml_base::{BaseError, CacheBudget, Cancel, FontSource, PartStore};
use std::sync::Arc;

pub struct NoFonts;

impl FontSource for NoFonts {
    fn face(&self, _: &FaceRequest) -> Result<Option<FaceData>, BaseError> {
        Ok(None)
    }
}

pub struct NoParts;

impl PartStore for NoParts {
    fn load(&self, _: &str) -> Result<Option<Arc<[u8]>>, BaseError> {
        Ok(None)
    }
    fn store(&self, _: &str, _: Arc<[u8]>) -> Result<(), BaseError> {
        Ok(())
    }
}

pub struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _: usize) -> bool {
        true
    }
    fn release(&self, _: usize) {}
    fn used(&self) -> usize {
        0
    }
}

pub struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

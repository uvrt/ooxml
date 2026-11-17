//! Owns: text bodies as the painter draws them -- wrapping ([`wrap`]), vertical measures
//! ([`metrics`]), a line's spans ([`line`](mod@line)) and a body in its frame ([`body`]).
//!
//! Must not: measure a glyph (the [`crate::measure`] port does).

pub mod body;
pub mod line;
pub mod metrics;
pub mod wrap;

pub use body::{format_auto_num, render_text_body, sp_autofit_height};

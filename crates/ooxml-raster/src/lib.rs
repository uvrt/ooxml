//! The scene's PNG adapter (docs/architecture/README.md, sections 3 and 8; layer 2, beside
//! `ooxml-render`): an SVG the typed writer wrote, drawn by resvg in process.
//!
//! The SVG is an output in its own right (the live view shows it, the tools return it), and
//! a PNG is one more step after it, never a replacement: [`rasterise`] takes the writer's
//! document as written. usvg 0.48 builds its tree only by parsing SVG (its node constructors
//! are crate-private), so the document is parsed here, in memory -- the exception to section
//! 8's "no SVG re-parse" that decision 11 records, until a usvg release with public
//! constructors is the one resvg-py ships too. Anything resvg needs to be different is
//! changed on the way in ([`FontSet`]'s database, [`RasterOptions`]), never in the SVG
//! handed out.
//!
//! Owns: the font database a drawing's text is drawn with ([`FontSet`], built once from the
//! files, folders and system fonts resvg-py is handed and shared by every drawing after),
//! the size a drawing is drawn at ([`Fit`]), the options resvg-py 0.5 parses with
//! ([`RasterOptions`]), and the drawing itself ([`rasterise`], [`Raster`]), pixel for pixel
//! resvg-py's `svg_to_bytes`.
//!
//! Must not: write or rewrite SVG, choose a face itself (resvg's database does, as in
//! Python), hold state between drawings beyond the [`FontSet`] its caller keeps, or depend
//! on an interpreter lock: every value here is `Send + Sync`.
#![forbid(unsafe_code)]

mod error;
pub mod fonts;
pub mod raster;

pub use error::RasterError;
pub use fonts::{Families, FontLoader, FontSet};
pub use raster::{rasterise, Fit, Raster, RasterOptions, Rgba};

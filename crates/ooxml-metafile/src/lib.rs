//! The metafile players (docs/architecture/README.md, sections 3 and 8; layer 2): EMF, EMF+
//! and WMF pictures played into a [`Scene`], with Windows PowerPoint as the reference, and
//! device-independent bitmaps decoded ([`dib`]).
//!
//! Owns: the record readers (EMF, its EMF+ comments, WMF), the GDI and GDI+ state machines
//! (device contexts, objects, map modes, world transforms, raster operations as drawn,
//! clipping, saved states and containers) and what they draw, as ooxml-common's
//! `metafile` package draws it -- the scene's SVG equals that package's, byte for byte but
//! for the deflate bytes of an embedded PNG (`png`).
//!
//! The input is untrusted (section 11): input size, records, objects, points, output,
//! decoded pixels, saved states and nesting are capped ([`MetafileLimits`]), every read is
//! bounds-checked, and nothing here panics for any bytes: a record that cannot be read is
//! named in [`Drawing::skipped`], a cap stops the playback and says so.
//!
//! Must not: write SVG itself (the scene's writer does), read files, or keep anything
//! between calls; it holds no global mutable state and relies on no interpreter lock.
#![forbid(unsafe_code)]

mod bytes;
mod dc;
pub mod dib;
mod emf;
mod error;
mod fonts;
mod geom;
mod player;
mod plus;
mod png;
mod pymath;
mod sha1;
mod wmf;

pub use error::ReadError;
pub use ooxml_base::MetafileLimits;
pub use ooxml_fonts::FontMapping;
use ooxml_scene::Scene;

use fonts::Faces;

/// How a metafile is drawn.
#[derive(Debug, Clone)]
pub struct Options {
    /// The font mapping its text's `font-family` stacks substitute faces with.
    pub font_mapping: FontMapping,
    /// The caps on the work it may ask for.
    pub limits: MetafileLimits,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            font_mapping: FontMapping::office_default(),
            limits: MetafileLimits::default(),
        }
    }
}

/// A metafile drawn, and what was left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawing {
    /// The drawing: its root's view box is the picture's frame in device units, stretched
    /// over the picture's box.
    pub scene: Scene,
    /// The picture's own size in pixels at 96 to the inch.
    pub width: f64,
    /// Likewise.
    pub height: f64,
    /// What was not drawn (or drawn approximately), by record or reason, with counts,
    /// sorted by name.
    pub skipped: Vec<(String, u64)>,
    /// The file carries EMF+ records, and they are what was drawn.
    pub emf_plus: bool,
    /// Its EMF+ records drew nothing, so its GDI records were drawn instead.
    pub gdi_fallback: bool,
    /// A cap stopped the playback before the end of the file.
    pub truncated: bool,
    /// How many elements were drawn; 0 when it draws nothing this player draws.
    pub drawn: u64,
}

impl Drawing {
    /// The drawing as a standalone SVG document, UTF-8.
    pub fn svg(&self) -> Vec<u8> {
        ooxml_scene::write_scene(&self.scene)
    }

    /// Every record was understood: nothing skipped, not a fallback, not truncated.
    pub fn complete(&self) -> bool {
        self.skipped.is_empty() && !self.gdi_fallback && !self.truncated
    }

    /// One sentence naming what was not drawn, or `None` when everything was.
    pub fn warning(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.skipped.is_empty() {
            let names: Vec<String> = self
                .skipped
                .iter()
                .map(|(name, count)| {
                    if *count > 1 {
                        format!("{name} (x{count})")
                    } else {
                        name.clone()
                    }
                })
                .collect();
            parts.push(format!("not drawn: {}", names.join(", ")));
        }
        if self.gdi_fallback {
            parts.push(
                "its EMF+ records draw nothing, so its GDI records are drawn instead (Windows draws only the EMF+ records)"
                    .to_owned(),
            );
        }
        if self.truncated {
            parts.push("it is larger than the player's limits, so the rest of it is not drawn".to_owned());
        }
        (!parts.is_empty()).then(|| parts.join("; "))
    }
}

/// Draws EMF or WMF bytes, or `None` when they are neither (or cannot be read at all).
pub fn draw(data: &[u8], options: &Options) -> Option<Drawing> {
    let faces = Faces::new(options.font_mapping.clone());
    draw_nested(data, &faces, options.limits, 0)
}

/// Whether `data` is an EMF or a WMF.
pub fn is_metafile(data: &[u8]) -> bool {
    emf::is_emf(data) || wmf::is_wmf(data)
}

/// The drawing of a metafile `nesting` EMF+ images deep.
pub(crate) fn draw_nested(data: &[u8], faces: &Faces, limits: MetafileLimits, nesting: u32) -> Option<Drawing> {
    let prefix = format!("m{}-", sha1::hex_prefix(data, 10));
    let (player, (width, height), emf_plus, gdi_fallback) = if emf::is_emf(data) {
        let played = emf::play(data, faces, limits, false, nesting)?;
        let size = played.header.natural_size();
        let mut player = played.player;
        let mut fallback = false;
        if played.emf_plus && player.drawn == 0 && !player.truncated {
            if let Some(again) = emf::play(data, faces, limits, true, nesting).filter(|a| a.player.drawn > 0) {
                player = again.player;
                fallback = true;
            }
        }
        (player, size, played.emf_plus, fallback)
    } else if wmf::is_wmf(data) {
        let played = wmf::play(data, faces, limits)?;
        (played.player, played.size, false, false)
    } else {
        return None;
    };
    let skipped: Vec<(String, u64)> = player.skipped.iter().map(|(k, v)| (k.clone(), *v)).collect();
    let truncated = player.truncated;
    // The last pending lines are drawn as the drawing is finished, and count.
    let (scene, drawn) = player.finish(width, height, prefix).ok()?;
    Some(Drawing {
        scene,
        width,
        height,
        skipped,
        emf_plus,
        gdi_fallback,
        truncated,
        drawn,
    })
}

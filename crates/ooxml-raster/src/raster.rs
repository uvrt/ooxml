//! Owns: one drawing -- an SVG document parsed by usvg in memory and drawn by resvg onto a
//! pixmap ([`rasterise`]) at a size ([`Fit`]) over a background, under the options
//! resvg-py 0.5's `svg_to_bytes` parses with ([`RasterOptions`]) -- and its pixels and PNG
//! ([`Raster`]).
//!
//! resvg-py's options, read from its source (`src/rust/lib.rs`, 0.5.0) and kept so the
//! pixels are its own: XML parsed with DTDs allowed, the document trimmed first; `dpi` 0
//! (usvg's absolute units -- `pt`, `in`, `mm` -- come out as zero, as in Python), font size
//! 16, no languages, an empty style sheet, shapes at geometric precision,
//! optimize-legibility text and optimize-quality images; a document without a size is
//! 100 by 100 (or the size asked for); fonts from the [`FontSet`] the caller keeps.
//!
//! Must not: write or change the SVG, or keep anything between drawings.

use crate::fonts::FontSet;
use crate::RasterError;
use resvg::tiny_skia::{IntSize, Pixmap, Transform};
use usvg::{roxmltree, ImageRendering, ShapeRendering, TextRendering};

/// A colour with alpha, 8 bits a channel, not premultiplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    /// Red.
    pub red: u8,
    /// Green.
    pub green: u8,
    /// Blue.
    pub blue: u8,
    /// Alpha: 255 is opaque.
    pub alpha: u8,
}

/// The size a drawing is drawn at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fit {
    /// The document's own size, in px.
    Original,
    /// This many pixels wide, the height in proportion.
    Width(u32),
    /// This many pixels high, the width in proportion.
    Height(u32),
    /// The document scaled to fit inside this box, keeping its proportions.
    Size(u32, u32),
    /// The document's own size times this.
    Zoom(f32),
}

impl Fit {
    fn size(self, size: IntSize) -> Option<IntSize> {
        match self {
            Fit::Original => Some(size),
            Fit::Width(w) => size.scale_to_width(w),
            Fit::Height(h) => size.scale_to_height(h),
            Fit::Size(w, h) => IntSize::from_wh(w, h).map(|s| size.scale_to(s)),
            Fit::Zoom(z) => size.scale_by(z),
        }
    }

    fn transform(self, size: IntSize) -> Transform {
        let from = size.to_size();
        let Some(to) = self.size(size).map(|s| s.to_size()) else {
            return Transform::default();
        };
        Transform::from_scale(to.width() / from.width(), to.height() / from.height())
    }

    /// The size a document without one is given (usvg's `default_size`), as resvg-py
    /// gives it for this fit.
    fn default_size(self) -> Result<usvg::Size, RasterError> {
        let positive = |value: u32, name: &'static str| {
            if value == 0 {
                Err(RasterError::Size(name))
            } else {
                Ok(value as f32)
            }
        };
        let (w, h) = match self {
            Fit::Size(w, h) => (positive(w, "width is zero")?, positive(h, "height is zero")?),
            Fit::Width(w) => (positive(w, "width is zero")?, 100.0),
            Fit::Height(h) => (100.0, positive(h, "height is zero")?),
            Fit::Zoom(z) if !z.is_finite() || z <= 0.0 => return Err(RasterError::Size("zoom is not positive")),
            Fit::Zoom(_) | Fit::Original => (100.0, 100.0),
        };
        usvg::Size::from_wh(w, h).ok_or(RasterError::Size("not a size"))
    }
}

/// How a drawing is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct RasterOptions {
    /// The size.
    pub fit: Fit,
    /// Painted behind the drawing; transparent where `None`.
    pub background: Option<Rgba>,
}

impl Default for RasterOptions {
    fn default() -> RasterOptions {
        RasterOptions {
            fit: Fit::Original,
            background: None,
        }
    }
}

/// A drawing's pixels.
#[derive(Debug, Clone)]
pub struct Raster {
    pixmap: Pixmap,
}

impl Raster {
    /// Pixels across.
    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    /// Pixels down.
    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    /// The pixels, row by row, RGBA with the colour premultiplied by alpha (resvg's own).
    pub fn premultiplied(&self) -> &[u8] {
        self.pixmap.data()
    }

    /// The drawing as PNG: resvg-py's bytes' pixels (tiny-skia's encoder, as resvg-py's).
    ///
    /// # Errors
    /// The encoder fails.
    pub fn png(&self) -> Result<Vec<u8>, RasterError> {
        self.pixmap.encode_png().map_err(|e| RasterError::Png(e.to_string()))
    }
}

fn usvg_options(fonts: &FontSet, fit: Fit) -> Result<usvg::Options<'static>, RasterError> {
    Ok(usvg::Options {
        resources_dir: None,
        dpi: 0.0,
        font_family: fonts.default_family().to_owned(),
        font_size: 16.0,
        languages: Vec::new(),
        shape_rendering: ShapeRendering::GeometricPrecision,
        text_rendering: TextRendering::OptimizeLegibility,
        image_rendering: ImageRendering::OptimizeQuality,
        default_size: fit.default_size()?,
        image_href_resolver: usvg::ImageHrefResolver::default(),
        font_resolver: usvg::FontResolver::default(),
        fontdb: fonts.database(),
        style_sheet: Some(String::new()),
    })
}

/// `svg` drawn with `fonts` as `options` ask: resvg-py's `svg_to_bytes`, pixel for pixel,
/// with the database built once instead of per call.
///
/// # Errors
/// The document is not XML, usvg cannot read it, or the size is zero.
pub fn rasterise(svg: &str, fonts: &FontSet, options: &RasterOptions) -> Result<Raster, RasterError> {
    let parsing = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    let document =
        roxmltree::Document::parse_with_options(svg.trim(), parsing).map_err(|e| RasterError::Xml(e.to_string()))?;
    let tree = usvg::Tree::from_xmltree(&document, &usvg_options(fonts, options.fit)?)
        .map_err(|e| RasterError::Svg(e.to_string()))?;
    let size = tree.size().to_int_size();
    let drawn = options.fit.size(size).ok_or(RasterError::Size("cannot scale"))?;
    let mut pixmap = Pixmap::new(drawn.width(), drawn.height()).ok_or(RasterError::Size("no pixmap"))?;
    if let Some(Rgba {
        red,
        green,
        blue,
        alpha,
    }) = options.background
    {
        pixmap.fill(resvg::tiny_skia::Color::from_rgba8(red, green, blue, alpha));
    }
    resvg::render(&tree, options.fit.transform(size), &mut pixmap.as_mut());
    Ok(Raster { pixmap })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="2" height="2" fill="#ff0000"/></svg>"##;

    #[test]
    fn a_drawing_is_its_own_size_and_colours() {
        let raster = rasterise(SQUARE, &FontSet::empty(), &RasterOptions::default()).unwrap();
        assert_eq!((raster.width(), raster.height()), (4, 2));
        assert_eq!(&raster.premultiplied()[..4], &[255, 0, 0, 255]);
        assert_eq!(&raster.premultiplied()[8..12], &[0, 0, 0, 0]);
        assert!(raster.png().unwrap().starts_with(b"\x89PNG"));
    }

    #[test]
    fn a_fit_scales_and_a_background_fills() {
        let options = RasterOptions {
            fit: Fit::Width(8),
            background: Some(Rgba {
                red: 0,
                green: 0,
                blue: 255,
                alpha: 255,
            }),
        };
        let raster = rasterise(SQUARE, &FontSet::empty(), &options).unwrap();
        assert_eq!((raster.width(), raster.height()), (8, 4));
        assert_eq!(&raster.premultiplied()[28..32], &[0, 0, 255, 255]);
    }

    #[test]
    fn what_cannot_be_drawn_is_an_error() {
        let fonts = FontSet::empty();
        assert!(matches!(
            rasterise("<svg", &fonts, &RasterOptions::default()),
            Err(RasterError::Xml(_))
        ));
        assert!(matches!(
            rasterise("<html/>", &fonts, &RasterOptions::default()),
            Err(RasterError::Svg(_))
        ));
        let zero = RasterOptions {
            fit: Fit::Zoom(0.0),
            background: None,
        };
        assert!(matches!(rasterise(SQUARE, &fonts, &zero), Err(RasterError::Size(_))));
    }
}

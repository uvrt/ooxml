//! Owns: how a drawing names a face its document name does not reach in a rasteriser that
//! files faces as resvg's font database does (docx2svg's `InstalledFonts.drawing_name`,
//! `DrawingName`, ooxml-common's `Face.rasteriser_families`, `Face.css`, `css_match` and
//! `weight_order`): the copy Word draws a family with (Word's bundle first), the family a
//! rasteriser files that file under (name ID 16 where it has one, else 1), and the CSS
//! weight, stretch and style that resolve to that file alone among the installed faces of
//! that family.
//!
//! Must not: lay anything out (the layout's faces are [`crate::WordFonts`]'), or write SVG.

use crate::catalogue::{Catalogue, FaceLoc, Folder, Location};
use crate::embedded::EmbeddedPart;
use crate::sfnt::Face;
use crate::store::FontStore;
use crate::substitutes::FontTable;
use crate::word::{Grade, LayoutFace, WordFontOptions, WordFonts};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// CSS's stretch keywords by `usWidthClass` (1 to 9).
const CSS_STRETCH: [&str; 9] = [
    "ultra-condensed",
    "extra-condensed",
    "condensed",
    "semi-condensed",
    "normal",
    "semi-expanded",
    "expanded",
    "extra-expanded",
    "ultra-expanded",
];

/// How a drawing names a face (`DrawingName`): the family a rasteriser files it under, and
/// the CSS weight, stretch and style that find it there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingName {
    /// The family.
    pub family: String,
    /// The weight, a multiple of 100.
    pub weight: u16,
    /// The stretch keyword.
    pub stretch: &'static str,
    /// The style: `normal`, `italic` or `oblique`.
    pub style: &'static str,
}

/// A face as a rasteriser matches it: where it is, and its `(usWeightClass, usWidthClass,
/// style)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CssFace {
    pub(crate) source: String,
    pub(crate) offset: usize,
    pub(crate) css: (u16, u16, &'static str),
}

fn u16_at(t: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*t.get(at)?, *t.get(at + 1)?]))
}

/// `(usWeightClass, usWidthClass, style)` as resvg's database reads them (`Face.css`).
pub(crate) fn css(face: &Face) -> (u16, u16, &'static str) {
    let Some(os2) = face.table(b"OS/2").filter(|t| t.len() >= 64) else {
        return (400, 5, "normal");
    };
    let weight = u16_at(os2, 4).unwrap_or(400);
    let width = u16_at(os2, 6).unwrap_or(5);
    let selection = u16_at(os2, 62).unwrap_or(0);
    let style = if selection & 0x01 != 0 {
        "italic"
    } else if selection & 0x200 != 0 {
        "oblique"
    } else {
        "normal"
    };
    (weight, if (1..=9).contains(&width) { width } else { 5 }, style)
}

/// The family names a rasteriser files a face under (`rasteriser_families`): name ID 16
/// where it has one, else 1, every language's, the English one first.
pub fn rasteriser_families(face: &Face) -> Vec<String> {
    let records = face.names();
    let id = if records.iter().any(|r| r.name_id == 16) { 16 } else { 1 };
    let mut chosen: Vec<_> = records.iter().filter(|r| r.name_id == id).collect();
    chosen.sort_by_key(|r| match (r.platform, r.language) {
        (3, 0x409) => 0,
        (1, 0) => 1,
        _ => 2,
    });
    let mut out: Vec<String> = Vec::new();
    for r in chosen {
        if !out.contains(&r.text) {
            out.push(r.text.clone());
        }
    }
    out
}

/// The weights tried for `desired`, best first (CSS Fonts 3, 5.2 step 4d).
fn weight_order(desired: u16, weights: &[u16]) -> Vec<u16> {
    let mut available: Vec<u16> = weights.to_vec();
    available.sort_unstable();
    available.dedup();
    let up = |pred: &dyn Fn(u16) -> bool| available.iter().copied().filter(|w| pred(*w)).collect::<Vec<_>>();
    let down = |pred: &dyn Fn(u16) -> bool| available.iter().rev().copied().filter(|w| pred(*w)).collect::<Vec<_>>();
    let mut out = Vec::new();
    if (400..=500).contains(&desired) {
        out.extend(up(&|w| desired <= w && w <= 500));
        out.extend(down(&|w| w < desired));
        out.extend(up(&|w| w > 500));
    } else if desired < 400 {
        out.extend(down(&|w| w <= desired));
        out.extend(up(&|w| w > desired));
    } else {
        out.extend(up(&|w| w >= desired));
        out.extend(down(&|w| w < desired));
    }
    out
}

/// The faces CSS font matching ends with among `faces` for `(stretch, style, weight)`.
pub(crate) fn css_match(faces: &[CssFace], stretch: u16, style: &str, weight: u16) -> Vec<CssFace> {
    let mut widths: Vec<u16> = faces.iter().map(|f| f.css.1).collect();
    widths.sort_unstable();
    widths.dedup();
    let narrower = widths.iter().rev().copied().find(|w| *w < stretch);
    let wider = widths.iter().copied().find(|w| *w > stretch);
    let chosen = if widths.contains(&stretch) {
        Some(stretch)
    } else if stretch <= 5 {
        narrower.or(wider)
    } else {
        wider.or(narrower)
    };
    let faces: Vec<&CssFace> = faces.iter().filter(|f| Some(f.css.1) == chosen).collect();
    let order: [&str; 3] = match style {
        "italic" => ["italic", "oblique", "normal"],
        "oblique" => ["oblique", "italic", "normal"],
        _ => ["normal", "oblique", "italic"],
    };
    let Some(chosen_style) = order.into_iter().find(|s| faces.iter().any(|f| f.css.2 == *s)) else {
        return Vec::new();
    };
    let faces: Vec<&CssFace> = faces.into_iter().filter(|f| f.css.2 == chosen_style).collect();
    let weights: Vec<u16> = faces.iter().map(|f| f.css.0).collect();
    let Some(&best) = weight_order(weight, &weights).first() else {
        return Vec::new();
    };
    faces.into_iter().filter(|f| f.css.0 == best).cloned().collect()
}

/// Every installed face as Word draws with it -- its bundle searched first -- and, read
/// when first asked for, by the family a rasteriser files it under: what does not depend on
/// a document, shared by a session's documents.
pub struct DrawingIndex {
    store: Arc<dyn FontStore>,
    catalogue: Catalogue,
    pool: Mutex<Option<HashMap<String, Vec<CssFace>>>>,
}

impl std::fmt::Debug for DrawingIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrawingIndex").finish_non_exhaustive()
    }
}

impl DrawingIndex {
    /// The faces of `folders` (in Word's order; its bundle is searched first here).
    pub fn new(store: Arc<dyn FontStore>, folders: &[Folder]) -> DrawingIndex {
        let mut ordered: Vec<Folder> = folders
            .iter()
            .filter(|f| f.location == Location::WordBundle)
            .cloned()
            .collect();
        ordered.extend(folders.iter().filter(|f| f.location != Location::WordBundle).cloned());
        let catalogue = Catalogue::scan(store.as_ref(), &ordered);
        DrawingIndex {
            store,
            catalogue,
            pool: Mutex::new(None),
        }
    }

    fn read_pool(&self) -> HashMap<String, Vec<CssFace>> {
        let mut out: HashMap<String, Vec<CssFace>> = HashMap::new();
        let mut faces: Vec<FaceLoc> = self.catalogue.faces();
        faces.sort_by(|a, b| {
            self.catalogue
                .file(a.file)
                .cmp(&self.catalogue.file(b.file))
                .then(a.number.cmp(&b.number))
        });
        for at in faces {
            let Some(path) = self.catalogue.file(at.file) else {
                continue;
            };
            let Some(face) = self
                .store
                .read(path)
                .ok()
                .and_then(|d| Face::new(d, at.number, path).ok())
            else {
                continue;
            };
            let entry = CssFace {
                source: path.to_owned(),
                offset: face.offset(),
                css: css(&face),
            };
            let mut names: Vec<String> = rasteriser_families(&face).iter().map(|n| n.to_lowercase()).collect();
            names.sort();
            names.dedup();
            for name in names {
                out.entry(name).or_default().push(entry.clone());
            }
        }
        out
    }

    fn pool_of(&self, family: &str) -> Vec<CssFace> {
        let mut pool = self.pool.lock().unwrap_or_else(PoisonError::into_inner);
        pool.get_or_insert_with(|| self.read_pool())
            .get(family)
            .cloned()
            .unwrap_or_default()
    }
}

/// The faces Word draws a document with and how a drawing names them.
pub struct DrawingFonts {
    fonts: WordFonts,
    index: Arc<DrawingIndex>,
}

impl std::fmt::Debug for DrawingFonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrawingFonts").finish_non_exhaustive()
    }
}

impl DrawingFonts {
    /// A document's drawing faces: `index`'s, with its embedded parts and font table.
    pub fn new(
        index: Arc<DrawingIndex>,
        embedded: &[EmbeddedPart],
        font_table: FontTable,
        options: WordFontOptions,
    ) -> DrawingFonts {
        let fonts = WordFonts::with_catalogue(
            Arc::clone(&index.store),
            index.catalogue.clone(),
            embedded,
            font_table,
            options,
        );
        DrawingFonts { fonts, index }
    }

    /// The face Word draws `family` in a style with (docx2svg's `drawing_face`): its bundle's
    /// copy first, then the installed ones, the document's embedded face, its substitute;
    /// `None` where no file holds it (a symbol face's recorded integers).
    pub fn drawing_face(&self, family: &str, bold: bool, italic: bool) -> Option<Arc<Face>> {
        match self.fonts.resolve(family, bold, italic).face? {
            LayoutFace::Sfnt(face) => Some(face),
            LayoutFace::Recorded(_) => None,
        }
    }

    /// How a drawing names `family` in a style; `None` where the document's name and style
    /// already reach the face Word draws, or no face is found.
    pub fn drawing_name(&self, family: &str, bold: bool, italic: bool) -> Option<DrawingName> {
        let resolution = self.fonts.resolve(family, bold, italic);
        let LayoutFace::Sfnt(face) = resolution.face? else {
            return None;
        };
        if resolution.grade == Grade::Missing {
            return None;
        }
        let families = rasteriser_families(&face);
        let first = families.first()?.clone();
        let reached = families.iter().any(|f| f.to_lowercase() == family.to_lowercase());
        if face.style().ok() == Some((bold, italic)) && reached {
            return None;
        }
        let own = CssFace {
            source: face.source().to_owned(),
            offset: face.offset(),
            css: css(&face),
        };
        let mut pool = vec![own.clone()];
        pool.extend(
            self.index
                .pool_of(&first.to_lowercase())
                .into_iter()
                .filter(|o| (o.source.as_str(), o.offset) != (own.source.as_str(), own.offset)),
        );
        let (weight_class, width, style) = own.css;
        let mut candidates: Vec<u16> = (1..10).map(|k| k * 100).collect();
        candidates.sort_by_key(|w| (w.abs_diff(weight_class), *w));
        let weight = candidates
            .iter()
            .copied()
            .find(|w| css_match(&pool, width, style, *w) == [own.clone()])
            .unwrap_or(candidates.first().copied().unwrap_or(400));
        let stretch = CSS_STRETCH
            .get(usize::from(width).saturating_sub(1))
            .copied()
            .unwrap_or("normal");
        Some(DrawingName {
            family: first,
            weight,
            stretch,
            style,
        })
    }
}

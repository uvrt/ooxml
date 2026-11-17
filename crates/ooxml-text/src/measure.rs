//! Owns: a paragraph measured -- its pieces (advances, automatic hyphens, kern pairs) and
//! its lines at a geometry -- and [`Measurer`], which memoises measures by content: the
//! paragraph's digest, the geometry's, the fonts epoch and the locale (design section 8),
//! through `ooxml-derive`'s [`Memo`] within a cache budget.
//!
//! Must not: hold a measure under anything but its content key (nothing is invalidated by
//! hand), or know where a paragraph is in a document.

use crate::auto::{auto_hyphens, RunInfo};
use crate::breaks::{break_pieces, Line};
use crate::geometry::Geometry;
use crate::glyphs::Glyphs;
use crate::input::{Digestible, Paragraph};
use crate::pieces::{hyphen_width, kern, paragraph_pieces, Piece};
use crate::units::Units;
use crate::TextError;
use ooxml_base::{Digest, DigestKey, Digester, Locale};
use ooxml_derive::{CacheBudget, Memo, MemoStats, Weigh};
use std::sync::Arc;

/// A paragraph's pieces and lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measured {
    /// Every piece, the label's first.
    pub pieces: Vec<Piece>,
    /// Every line.
    pub lines: Vec<Line>,
}

/// The paragraph's pieces: advances, automatic hyphens and kern pairs.
pub fn pieces(glyphs: &dyn Glyphs, paragraph: &Paragraph) -> Result<Vec<Piece>, TextError> {
    let mut items = paragraph_pieces(glyphs, paragraph)?;
    if let Some(hyphenation) = paragraph.hyphenation {
        let mut runs = RunInfo::new();
        for (index, run) in paragraph.runs.iter().enumerate().filter(|(_, r)| !r.vanish) {
            runs.insert(
                index as i64,
                (run.lang.clone(), run.no_proof, hyphen_width(glyphs, run)),
            );
        }
        items = auto_hyphens(items, &runs, hyphenation.mode15, hyphenation.no_caps);
    }
    kern(glyphs, &mut items)?;
    Ok(items)
}

/// The paragraph measured and broken into lines at `geometry`; `columns`: it is in a
/// section of several columns.
pub fn measure(
    glyphs: &dyn Glyphs,
    paragraph: &Paragraph,
    geometry: &Geometry,
    columns: bool,
) -> Result<Measured, TextError> {
    let pieces = pieces(glyphs, paragraph)?;
    let lines = break_pieces(&pieces, geometry, columns);
    Ok(Measured { pieces, lines })
}

/// What a measure is memoised under.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MeasureKey {
    /// The paragraph's content.
    pub paragraph: Digest,
    /// The geometry's, with the columns flag.
    pub geometry: Digest,
    /// The fonts epoch.
    pub fonts: Digest,
    /// The locale's tag.
    pub locale: String,
}

fn units(d: &mut Digester, value: Units) {
    let (n, den) = value.parts();
    d.update(&n.to_le_bytes());
    d.update(&den.to_le_bytes());
}

fn geometry_digest(key: DigestKey, geometry: &Geometry, columns: bool) -> Digest {
    let mut d = Digester::new(key);
    for value in [
        geometry.first_start,
        geometry.start,
        geometry.right,
        geometry.default_tab,
        geometry.tab_origin,
    ] {
        units(&mut d, value);
    }
    d.update(&(geometry.tabs.len() as u64).to_le_bytes());
    for (position, align) in &geometry.tabs {
        units(&mut d, *position);
        d.update(&[*align as u8]);
    }
    for value in [geometry.hanging_stop, geometry.hyphen_zone, geometry.hyphen_gap] {
        match value {
            Some(v) => {
                d.update(&[1]);
                units(&mut d, v);
            }
            None => d.update(&[0]),
        }
    }
    d.update(&[
        u8::from(geometry.squeeze),
        u8::from(geometry.hyphen_next),
        u8::from(columns),
    ]);
    d.update(&geometry.hyphen_limit.to_le_bytes());
    d.finish()
}

/// A memoised outcome: a measure, or why the paragraph is unmeasurable.
#[derive(Debug, Clone)]
struct Outcome(Result<Arc<Measured>, TextError>);

impl Weigh for Outcome {
    fn weight(&self) -> usize {
        match &self.0 {
            Ok(m) => {
                std::mem::size_of::<Measured>()
                    + m.pieces.capacity() * std::mem::size_of::<Piece>()
                    + m.lines.capacity() * std::mem::size_of::<Line>()
            }
            Err(TextError::Unmeasurable(message)) => message.len() + std::mem::size_of::<TextError>(),
        }
    }
}

/// Measures memoised by content, within a cache budget. One per document's layout (or
/// per session); it takes `&mut self`, so a parallel layout measures its misses with
/// [`measure`] and keeps them with [`Measurer::keep`].
pub struct Measurer {
    memo: Memo<MeasureKey, Outcome>,
    key: DigestKey,
}

impl Measurer {
    /// A measurer reserving from `budget`, digesting content under `key` (a session's
    /// random key where content is untrusted).
    pub fn new(budget: Arc<dyn CacheBudget>, key: DigestKey) -> Measurer {
        Measurer {
            memo: Memo::new(budget),
            key,
        }
    }

    /// The key a paragraph's measure at a geometry is memoised under.
    pub fn key(
        &self,
        glyphs: &dyn Glyphs,
        paragraph: &Paragraph,
        geometry: &Geometry,
        columns: bool,
        locale: &Locale,
    ) -> MeasureKey {
        let mut d = Digester::new(self.key);
        paragraph.feed(&mut d);
        MeasureKey {
            paragraph: d.finish(),
            geometry: geometry_digest(self.key, geometry, columns),
            fonts: glyphs.epoch(),
            locale: locale.tag().to_owned(),
        }
    }

    /// The paragraph measured at `geometry`, from the memo where it holds it.
    pub fn measure(
        &mut self,
        glyphs: &dyn Glyphs,
        paragraph: &Paragraph,
        geometry: &Geometry,
        columns: bool,
        locale: &Locale,
    ) -> Result<Arc<Measured>, TextError> {
        let key = self.key(glyphs, paragraph, geometry, columns, locale);
        let outcome = self.memo.get_or_compute(&key, || {
            Outcome(measure(glyphs, paragraph, geometry, columns).map(Arc::new))
        });
        outcome.0
    }

    /// The measure held under `key`, counted as a use.
    pub fn get(&mut self, key: &MeasureKey) -> Option<Result<Arc<Measured>, TextError>> {
        self.memo.get(key).map(|o| o.0)
    }

    /// Keep a measure computed elsewhere under its key.
    pub fn keep(&mut self, key: MeasureKey, measured: Result<Arc<Measured>, TextError>) -> bool {
        self.memo.insert(key, Outcome(measured))
    }

    /// The memo's counters.
    pub fn stats(&self) -> MemoStats {
        self.memo.stats()
    }
}

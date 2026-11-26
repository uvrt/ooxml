//! Owns: what laying a version out makes beside its layout -- its pictures and drawings'
//! painter, what it says (warnings and approximations), the facts it carries from the
//! version before -- and a generation's fresh layout engine.
//!
//! Must not: lay out (its parent does) or keep anything itself.

use super::{Generation, KEY};
use crate::drawing_said::DrawingWarnings;
use crate::drawings::WordDrawings;
use crate::fonts::DocumentFonts;
use crate::pictures::{PackagePictures, PictureMemo};
use crate::version::LaidVersion;
use ooxml_base::Locale;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_word_layout::paint::GlyphSize;
use ooxml_word_layout::tables::WordTables;
use ooxml_word_layout::WordLayout;
use std::sync::Arc;

/// A version's pictures (its package less its text parts, its metafiles' drawings kept in
/// `memo`) and its drawings' painter, writing charts' numbers in `numbers`.
pub(super) fn pictures_of(
    package: &Package,
    document: &ooxml_word_model::WordDocument,
    fonts: &DocumentFonts,
    numbers: &ooxml_numfmt::Locale,
    memo: &Arc<PictureMemo>,
) -> (Option<PackagePictures>, Option<WordDrawings>) {
    let pictures = PackagePictures::new(package).map(|p| p.with_memo(Arc::clone(memo)));
    let drawings = pictures.clone().map(|p| {
        let inks: Arc<dyn crate::drawings::vml::Inks> = Arc::clone(&fonts.inks);
        WordDrawings::new(document, p, numbers.clone(), Some(inks))
    });
    (pictures, drawings)
}

/// What a version's warnings are said with, beyond its engine generation.
pub(super) struct SayWith<'a> {
    pub(super) locale: &'a Locale,
    pub(super) glyph_size: GlyphSize,
    pub(super) decimal: char,
    /// A part beside the main one changed since the version before.
    pub(super) others_changed: bool,
    /// A notes', header's or footer's part is not the one the model opened before (read
    /// from the packages, not from what the caller said changed).
    pub(super) drawn_changed: bool,
}

/// What `laid` says (its warnings and approximations); the pages the version before
/// (`before`) kept said again as they were, and their first pages taken from it.
pub(super) fn say(
    generation: &mut Generation,
    (laid, before): (&ooxml_word_layout::DocumentLayout, Option<&LaidVersion>),
    drawings: Option<&WordDrawings>,
    with: &SayWith<'_>,
) -> crate::warnings::Said {
    if with.others_changed {
        // A part beside the main one changed (a chart's, a picture's, a story's): what a
        // kept page's drawings said may have changed with it.
        generation.said = crate::page_said::PageMemo::default();
        generation.epoch += 1;
    } else if with.drawn_changed {
        // A page laid out the same may draw other notes or stories.
        generation.epoch += 1;
    }
    let mut painter = DrawingWarnings {
        layout: &mut generation.layout,
        fonts: &generation.fonts,
        drawings,
        glyph_size: with.glyph_size,
        decimal: with.decimal,
        text_paths: Vec::new(),
    };
    crate::warnings::said(
        &mut generation.model,
        (laid, before.map(|b| &b.layout)),
        &mut painter,
        with.locale,
        &mut generation.said,
    )
}

/// What the facts of a version laid out from `previous` are made from, when first asked:
/// `previous`, where its facts were made (the pages it kept take theirs), else what
/// `previous`'s were to be made from -- at most one version, whose facts are made, held.
pub(super) fn facts_source(previous: Option<&Arc<LaidVersion>>) -> Option<Arc<LaidVersion>> {
    let previous = previous?;
    if previous.has_facts() {
        Some(Arc::clone(previous))
    } else {
        previous.facts_source()
    }
}

/// A layout engine with no memos, measuring with `fonts`.
pub(super) fn new_layout(fonts: &DocumentFonts, locale: &Locale, budget: &Arc<dyn CacheBudget>) -> WordLayout {
    let faces: Arc<dyn ooxml_word_layout::Faces> = Arc::clone(&fonts.glyphs) as Arc<dyn ooxml_word_layout::Faces>;
    WordLayout::new(faces, Arc::new(WordTables), locale.clone(), budget, KEY)
}

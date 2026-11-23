//! Owns: what laying a version out makes beside its layout -- its body ids (found from the
//! version before's), its pictures and drawings' painter, what it says (warnings and
//! approximations), the facts it carries from the version before -- and a generation's
//! fresh layout engine.
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

/// The version's body ids: the main part's ids found from the last version's
/// (`PartIds::after`), its blocks named when asked.
pub(super) fn version_ids(generation: &mut Generation, package: &Package, main: &str) -> crate::ids::VersionIds {
    let Ok(Some(tree)) = package.tree(main) else {
        generation.ids = None;
        return crate::ids::VersionIds::default();
    };
    let root = Arc::clone(tree.root());
    let ids = match generation.ids.take() {
        Some((old, ids)) if Arc::ptr_eq(&old, &root) => ids,
        Some((old, ids)) => Arc::new(ids.after(&old, &root)),
        None => Arc::new(ooxml_word_model::ids::PartIds::new("body", &root)),
    };
    generation.ids = Some((Arc::clone(&root), Arc::clone(&ids)));
    crate::ids::VersionIds::new(root, ids)
}

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

/// What `laid` says (its warnings and approximations), its facts made into `facts` where
/// saying them needs them; the pages the version before kept said again as they were.
pub(super) fn say(
    generation: &mut Generation,
    laid: &ooxml_word_layout::DocumentLayout,
    drawings: Option<&WordDrawings>,
    with: &SayWith<'_>,
    facts: &std::sync::OnceLock<ooxml_word_layout::facts::LayoutFacts>,
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
        laid,
        &mut painter,
        with.locale,
        facts,
        &mut generation.said,
    )
}

/// The facts of `laid`, made from the version it was laid out from where that one's were
/// made (the pages it kept take theirs), else left to be made when first asked.
pub(super) fn carried_facts(
    previous: Option<&LaidVersion>,
    laid: &ooxml_word_layout::DocumentLayout,
) -> std::sync::OnceLock<ooxml_word_layout::facts::LayoutFacts> {
    match previous.and_then(|p| Some((&p.layout, p.facts.get()?))) {
        Some((before, known)) => {
            std::sync::OnceLock::from(ooxml_word_layout::facts::LayoutFacts::after(laid, before, known))
        }
        None => std::sync::OnceLock::new(),
    }
}

/// A layout engine with no memos, measuring with `fonts`.
pub(super) fn new_layout(fonts: &DocumentFonts, locale: &Locale, budget: &Arc<dyn CacheBudget>) -> WordLayout {
    let faces: Arc<dyn ooxml_word_layout::Faces> = Arc::clone(&fonts.glyphs) as Arc<dyn ooxml_word_layout::Faces>;
    WordLayout::new(faces, Arc::new(WordTables), locale.clone(), budget, KEY)
}

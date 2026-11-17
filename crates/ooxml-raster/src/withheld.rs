//! Owns: a drawing's texts whose faces are chosen with some of the set's files left out
//! ([`Withheld`]). docx2svg's drawing plan draws a text whose face loses a tie to a face of
//! another family (two faces resvg's database files under one name) in a pass of its own,
//! without the rival's files; here the passes are one drawing, and such a text is given
//! the face resvg's own selector chooses in the set less those files.
//!
//! Must not: choose a face itself (resvg's selector does, over the narrower set), or change
//! the SVG.

use ooxml_fonts::plan::TextQuery;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// Per text query, the files of the set left out when its face is chosen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Withheld {
    /// Each text's query, and the files left out for it.
    pub texts: HashMap<TextQuery, BTreeSet<String>>,
}

/// What a text asks for, as usvg reads its font.
pub(crate) fn query_of(font: &usvg::Font) -> TextQuery {
    let stretch = match font.stretch() {
        usvg::FontStretch::UltraCondensed => 1,
        usvg::FontStretch::ExtraCondensed => 2,
        usvg::FontStretch::Condensed => 3,
        usvg::FontStretch::SemiCondensed => 4,
        usvg::FontStretch::Normal => 5,
        usvg::FontStretch::SemiExpanded => 6,
        usvg::FontStretch::Expanded => 7,
        usvg::FontStretch::ExtraExpanded => 8,
        usvg::FontStretch::UltraExpanded => 9,
    };
    TextQuery {
        families: font
            .families()
            .iter()
            .filter_map(|f| match f {
                usvg::FontFamily::Named(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        stretch,
        style: match font.style() {
            usvg::FontStyle::Normal => "normal",
            usvg::FontStyle::Italic => "italic",
            usvg::FontStyle::Oblique => "oblique",
        }
        .to_owned(),
        weight: font.weight(),
    }
}

/// The path a face was read from, where it was read from a file.
pub(crate) fn source_path(source: &fontdb::Source) -> Option<&str> {
    match source {
        fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => path.to_str(),
        fontdb::Source::Binary(_) => None,
    }
}

impl Withheld {
    /// The narrower sets, one per distinct set of files left out (a face keeps its id).
    fn databases(&self, full: &fontdb::Database) -> HashMap<BTreeSet<String>, Arc<fontdb::Database>> {
        let mut out = HashMap::new();
        for files in self.texts.values().filter(|f| !f.is_empty()) {
            if out.contains_key(files) {
                continue;
            }
            let mut narrower = full.clone();
            let gone: Vec<fontdb::ID> = full
                .faces()
                .filter(|f| source_path(&f.source).is_some_and(|p| files.contains(p)))
                .map(|f| f.id)
                .collect();
            for id in gone {
                narrower.remove_face(id);
            }
            out.insert(files.clone(), Arc::new(narrower));
        }
        out
    }

    /// usvg's font resolver choosing each text's face in its narrower set.
    pub(crate) fn resolver(&self, full: &fontdb::Database) -> usvg::FontResolver<'static> {
        let databases = self.databases(full);
        let texts = self.texts.clone();
        let select = usvg::FontResolver::default_font_selector();
        usvg::FontResolver {
            select_font: Box::new(move |font, database| {
                let narrower = texts.get(&query_of(font)).and_then(|files| databases.get(files));
                match narrower {
                    Some(narrower) => select(font, &mut Arc::clone(narrower)),
                    None => select(font, database),
                }
            }),
            select_fallback: usvg::FontResolver::default_fallback_selector(),
        }
    }
}

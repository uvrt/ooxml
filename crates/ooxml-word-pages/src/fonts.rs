//! Owns: the fonts a document is laid out and drawn with -- the machine's folders scanned
//! once ([`FontSetup`]: Word's catalogue for measuring, the drawing index for naming faces
//! in SVG), and per document its font table's classes and its embedded faces read from
//! its package ([`DocumentFonts`]), as docx2svg's `InstalledFonts` and `drawing_name` find
//! them.
//!
//! Must not: choose the folders (the caller passes docx2svg's, in Word's order) or read a
//! font file beyond what the fonts layer asks for.

use ooxml_base::Digest;
use ooxml_fonts::{
    Catalogue, DrawingFonts, DrawingIndex, DrawingName, EmbeddedPart, Folder, FontStore, FontTable, FontTableEntry,
    WordFontOptions, WordFonts,
};
use ooxml_opc::Package;
use ooxml_word_layout::paint::FaceNames;
use ooxml_word_layout::WordGlyphs;
use ooxml_word_model::xmlread::{attr, child, children, local};
use std::sync::Arc;

/// The machine's fonts for one set of folders and options: scanned once, shared by every
/// document laid out with them.
pub struct FontSetup {
    store: Arc<dyn FontStore>,
    catalogue: Catalogue,
    index: Arc<DrawingIndex>,
    options: WordFontOptions,
}

impl std::fmt::Debug for FontSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontSetup")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl FontSetup {
    /// The faces of `folders` (Word's order) read through `store`, laid out with `options`.
    pub fn new(store: Arc<dyn FontStore>, folders: &[Folder], options: WordFontOptions) -> FontSetup {
        let catalogue = Catalogue::scan(store.as_ref(), folders);
        let index = Arc::new(DrawingIndex::new(Arc::clone(&store), folders));
        FontSetup {
            store,
            catalogue,
            index,
            options,
        }
    }

    /// The fonts of the document in `package`: these faces, with its font table and its
    /// embedded faces.
    pub fn document(&self, package: &Package) -> DocumentFonts {
        let table = font_table(package);
        let embedded = embedded_parts(package);
        let fonts = WordFonts::with_catalogue(
            Arc::clone(&self.store),
            self.catalogue.clone(),
            &embedded,
            table.clone(),
            self.options.clone(),
        );
        let epoch = fonts.epoch();
        let names = DrawingFonts::new(Arc::clone(&self.index), &embedded, table, self.options.clone());
        DocumentFonts {
            glyphs: Arc::new(WordGlyphs::new(fonts)),
            names: Arc::new(Names(names)),
            epoch,
        }
    }
}

/// A document's fonts: what its layout measures with, and how its SVG names faces.
#[derive(Clone)]
pub struct DocumentFonts {
    /// The faces, as the layout's port.
    pub glyphs: Arc<WordGlyphs>,
    /// How the SVG names faces.
    pub names: Arc<dyn FaceNames>,
    /// What measures under these fonts are keyed by: a change means a new engine.
    pub epoch: Digest,
}

impl std::fmt::Debug for DocumentFonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentFonts")
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

/// Faces named as docx2svg names them on this machine (`drawing_name`).
struct Names(DrawingFonts);

impl FaceNames for Names {
    fn drawing_name(&self, family: &str, bold: bool, italic: bool) -> Option<DrawingName> {
        self.0.drawing_name(family, bold, italic)
    }
}

/// Where docx2svg reads the font table: this part, whatever the main part's name
/// (`font_table_classes`, `embedded_faces`).
const FONT_TABLE: &str = "word/fontTable.xml";

/// `word/fontTable.xml`'s `w:family` and `w:pitch` per face, by its name lowercased
/// (docx2svg's `font_table_classes`); empty where the part is missing or unreadable.
pub fn font_table(package: &Package) -> FontTable {
    let mut out = FontTable::new();
    let Ok(Some(tree)) = package.tree(FONT_TABLE) else {
        return out;
    };
    for font in children(tree.root(), "font") {
        let Some(name) = attr(font, "name") else { continue };
        let value = |what: &str| child(font, what).and_then(|e| attr(e, "val")).map(str::to_owned);
        out.insert(
            name.to_lowercase(),
            FontTableEntry {
                family: value("family"),
                pitch: value("pitch"),
            },
        );
    }
    out
}

/// Every embedded face of the font table, with its part's bytes as stored (docx2svg's
/// `embedded_faces`): `w:embedRegular`, `w:embedBold`, ... with a relationship to a part
/// the package has.
pub fn embedded_parts(package: &Package) -> Vec<EmbeddedPart> {
    let mut out = Vec::new();
    let Ok(Some(tree)) = package.tree(FONT_TABLE) else {
        return out;
    };
    let Ok(rels) = package.relationships(FONT_TABLE) else {
        return out;
    };
    for font in children(tree.root(), "font") {
        let Some(name) = attr(font, "name") else { continue };
        for embed in ooxml_word_model::xmlread::elements(font) {
            let Some(style) = local(embed).strip_prefix("embed") else {
                continue;
            };
            let (Some(id), Some(key)) = (attr(embed, "id"), attr(embed, "fontKey")) else {
                continue;
            };
            let Some(target) = rels.get(id).filter(|r| !r.external).and_then(|r| r.target_part.clone()) else {
                continue;
            };
            let Ok(Some(bytes)) = package.read(&target) else {
                continue;
            };
            out.push(EmbeddedPart {
                family: name.to_owned(),
                style: style.to_owned(),
                key: key.trim_matches(|c| c == '{' || c == '}').to_owned(),
                bytes: bytes.to_vec(),
            });
        }
    }
    out
}

//! Owns: the face Word lays a run out with, and how good it is -- docx2svg's
//! `InstalledFonts.face`: installed faces by name ([`Catalogue`]), then the document's
//! embedded ones, then the chain of substitutes named for it, then a recorded symbol face,
//! then (as an approximation, reported) a default or generic substitute -- and the advance
//! of a character the face lacks, from the faces Word falls back to (`_glyph_fallback`).
//! Every answer carries its [`Grade`], so the caller reports substitutions (design rule 8)
//! without this object recording anything.
//!
//! Must not: know about runs or paragraphs, or decide a character's slot or size.

use crate::catalogue::{lookup, Catalogue, FaceLoc, Folder, EPOCH_KEY};
use crate::embedded::{embed_style, embedded_face, EmbeddedPart};
use crate::recorded::{recorded_face, RecordedFace};
use crate::sfnt::{Face, FaceMetrics};
use crate::store::FontStore;
use crate::substitutes::{
    default_substitutes, face_class, generic_fallbacks, FaceClass, FontTable, SubstitutionKind, GENERIC_FALLBACKS,
    WORD_SUBSTITUTES,
};
use ooxml_base::{Digest, Digester};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// What a Word layout is configured with besides its folders (docx2svg's `ConvertOptions`
/// as `_fonts` reads it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordFontOptions {
    /// Face (lowercased) -> the face used in its place where it is absent.
    pub substitutes: Vec<(String, String)>,
    /// Lay an absent Symbol or Wingdings out from its recorded integers.
    pub recorded_faces: bool,
    /// Lay an absent face with no substitute named out with a default or generic one.
    pub fallbacks: bool,
}

impl Default for WordFontOptions {
    fn default() -> Self {
        WordFontOptions {
            substitutes: WORD_SUBSTITUTES
                .iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect(),
            recorded_faces: true,
            fallbacks: true,
        }
    }
}

/// A substitution: what was asked for, what was used, and how good it is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Substitution {
    /// The family the document names.
    pub family: String,
    /// The family laid out in its place.
    pub substitute: String,
    /// Whether the two were measured to the same advances and line metrics.
    pub same_metrics: bool,
    /// How the substitute was chosen.
    pub kind: SubstitutionKind,
    /// For a generic substitute, the class it was chosen by.
    pub class: Option<FaceClass>,
}

/// How a face was found: the grade a coverage report gives it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Grade {
    /// Installed under the name asked for.
    Installed,
    /// Embedded in the document.
    Embedded,
    /// Another face, laid out in its place.
    Substituted(Substitution),
    /// Laid out from the integers recorded from Word's copy (no face to draw with).
    Recorded,
    /// Found nowhere, with no substitute.
    Missing,
}

/// A face a layout measures with.
#[derive(Debug, Clone)]
pub enum LayoutFace {
    /// A face read from a file or an embedded part.
    Sfnt(Arc<Face>),
    /// A symbol face's recorded integers.
    Recorded(&'static RecordedFace),
}

impl LayoutFace {
    /// Units per em.
    pub fn units_per_em(&self) -> u16 {
        match self {
            LayoutFace::Sfnt(face) => face.units_per_em(),
            LayoutFace::Recorded(face) => face.metrics.units_per_em,
        }
    }

    /// The advance of `ch` in font units, if the face has it.
    pub fn advance(&self, ch: char) -> Option<u16> {
        match self {
            LayoutFace::Sfnt(face) => face.advance(ch),
            LayoutFace::Recorded(face) => face.advance(ch),
        }
    }

    /// The legacy kern adjustment between two characters (a recorded face has none).
    pub fn kern(&self, left: char, right: char) -> i16 {
        match self {
            LayoutFace::Sfnt(face) => face.kern(left, right),
            LayoutFace::Recorded(_) => 0,
        }
    }

    /// The line and script metrics.
    pub fn metrics(&self) -> Option<FaceMetrics> {
        match self {
            LayoutFace::Sfnt(face) => face.metrics().ok(),
            LayoutFace::Recorded(face) => Some(face.metrics),
        }
    }
}

/// The face a family resolved to, with its grade.
#[derive(Debug, Clone)]
pub struct Resolution {
    /// The face, or `None` for a [`Grade::Missing`] one.
    pub face: Option<LayoutFace>,
    /// How it was found.
    pub grade: Grade,
}

/// An advance and the units per em it is in, with the face it was taken from where the
/// run's face lacks the character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphAdvance {
    /// The advance, font units.
    pub advance: u16,
    /// Units per em of the face it was taken from.
    pub units_per_em: u16,
    /// The face it was taken from (or `.notdef`, the face's missing glyph), where the
    /// run's face lacks the character.
    pub fallback: Option<String>,
}

type Key = (String, bool, bool);

#[derive(Default)]
struct Caches {
    resolved: HashMap<Key, Resolution>,
    loaded: HashMap<FaceLoc, Option<Arc<Face>>>,
    defaults: HashMap<Key, Option<(FaceLoc, Substitution)>>,
    glyphs: HashMap<(Key, char), Option<GlyphAdvance>>,
}

/// The faces Word lays a document out with: the installed ones, then the document's own.
/// Shared by a session's documents (it caches faces as it reads them; safe to use from
/// several threads).
pub struct WordFonts {
    store: Arc<dyn FontStore>,
    catalogue: Catalogue,
    embedded: HashMap<Key, Arc<Face>>,
    options: WordFontOptions,
    font_table: FontTable,
    epoch: Digest,
    caches: Mutex<Caches>,
}

impl std::fmt::Debug for WordFonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WordFonts")
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

impl WordFonts {
    /// The fonts of `folders` (in Word's order) read through `store`, with a document's
    /// embedded parts and font table classes.
    pub fn new(
        store: Arc<dyn FontStore>,
        folders: &[Folder],
        embedded: &[EmbeddedPart],
        font_table: FontTable,
        options: WordFontOptions,
    ) -> WordFonts {
        let catalogue = Catalogue::scan(store.as_ref(), folders);
        WordFonts::with_catalogue(store, catalogue, embedded, font_table, options)
    }

    /// As [`WordFonts::new`], over a catalogue already scanned (a session scans once).
    pub fn with_catalogue(
        store: Arc<dyn FontStore>,
        catalogue: Catalogue,
        embedded: &[EmbeddedPart],
        font_table: FontTable,
        options: WordFontOptions,
    ) -> WordFonts {
        let mut digest = Digester::new(EPOCH_KEY);
        digest.update(&catalogue.epoch().0);
        let mut faces = HashMap::new();
        for part in embedded {
            digest.update(part.family.as_bytes());
            digest.update(part.style.as_bytes());
            digest.update(&part.bytes);
            if let Ok(face) = embedded_face(part) {
                let (bold, italic) = embed_style(&part.style);
                faces.insert((part.family.to_lowercase(), bold, italic), Arc::new(face));
            }
        }
        for (from, to) in &options.substitutes {
            digest.update(from.as_bytes());
            digest.update(to.as_bytes());
        }
        digest.update(&[u8::from(options.recorded_faces), u8::from(options.fallbacks)]);
        let mut table: Vec<_> = font_table.iter().collect();
        table.sort_by(|a, b| a.0.cmp(b.0));
        for (name, entry) in table {
            digest.update(name.as_bytes());
            digest.update(format!("{:?}{:?}", entry.family, entry.pitch).as_bytes());
        }
        WordFonts {
            store,
            catalogue,
            embedded: faces,
            options,
            font_table,
            epoch: digest.finish(),
            caches: Mutex::new(Caches::default()),
        }
    }

    /// What a measure cached under these fonts is keyed by: the folders, files and their
    /// stamps, the embedded faces, the substitutes and the font table.
    pub fn epoch(&self) -> Digest {
        self.epoch
    }

    fn caches(&self) -> impl std::ops::DerefMut<Target = Caches> + '_ {
        self.caches.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn load(&self, at: FaceLoc) -> Option<Arc<Face>> {
        if let Some(found) = self.caches().loaded.get(&at) {
            return found.clone();
        }
        let path = self.catalogue.file(at.file)?;
        let face = self
            .store
            .read(path)
            .ok()
            .and_then(|data| Face::new(data, at.number, path).ok())
            .map(Arc::new);
        self.caches().loaded.insert(at, face.clone());
        face
    }

    /// The face Word lays `family` out with in a style, and its grade.
    pub fn resolve(&self, family: &str, bold: bool, italic: bool) -> Resolution {
        let key = (family.to_owned(), bold, italic);
        if let Some(found) = self.caches().resolved.get(&key) {
            return found.clone();
        }
        let found = self.find(family, bold, italic);
        self.caches().resolved.insert(key, found.clone());
        found
    }

    fn installed(&self, at: FaceLoc, grade: Grade) -> Resolution {
        match self.load(at) {
            Some(face) => Resolution {
                face: Some(LayoutFace::Sfnt(face)),
                grade,
            },
            None => Resolution {
                face: None,
                grade: Grade::Missing,
            },
        }
    }

    fn find(&self, family: &str, bold: bool, italic: bool) -> Resolution {
        let missing = Resolution {
            face: None,
            grade: Grade::Missing,
        };
        if family.is_empty() {
            return missing;
        }
        if let Some(at) = self.catalogue.lookup(family, bold, italic) {
            return self.installed(at, Grade::Installed);
        }
        if let Some(face) = lookup(&self.embedded, family, bold, italic) {
            return Resolution {
                face: Some(LayoutFace::Sfnt(face)),
                grade: Grade::Embedded,
            };
        }
        if let Some((at, substitution)) = self.named(family, bold, italic) {
            return self.installed(at, Grade::Substituted(substitution));
        }
        if self.options.recorded_faces {
            if let Some(face) = recorded_face(family) {
                return Resolution {
                    face: Some(LayoutFace::Recorded(face)),
                    grade: Grade::Recorded,
                };
            }
        }
        if self.options.fallbacks {
            if let Some((at, substitution)) = self.default(family, bold, italic) {
                return self.installed(at, Grade::Substituted(substitution));
            }
        }
        missing
    }

    /// The first installed face of the chain of substitutes named for `family`.
    fn named(&self, family: &str, bold: bool, italic: bool) -> Option<(FaceLoc, Substitution)> {
        let substitute = |name: &str| {
            let key = name.to_lowercase();
            self.options
                .substitutes
                .iter()
                .find(|(from, _)| *from == key)
                .map(|(_, to)| to.clone())
        };
        let mut seen = vec![family.to_lowercase()];
        let mut name = substitute(family);
        while let Some(current) = name.filter(|n| !n.is_empty() && !seen.contains(&n.to_lowercase())) {
            if let Some(at) = self.catalogue.lookup(&current, bold, italic) {
                let measured = WORD_SUBSTITUTES
                    .iter()
                    .find(|(office, _)| *office == family.to_lowercase());
                let compatible = measured.is_some_and(|(_, s)| s.to_lowercase() == current.to_lowercase());
                return Some((
                    at,
                    Substitution {
                        family: family.to_owned(),
                        substitute: current,
                        same_metrics: compatible,
                        kind: SubstitutionKind::Named,
                        class: None,
                    },
                ));
            }
            seen.push(current.to_lowercase());
            name = substitute(&current);
        }
        None
    }

    /// The first installed default or generic substitute of `family` (docx2svg's `_default`).
    fn default(&self, family: &str, bold: bool, italic: bool) -> Option<(FaceLoc, Substitution)> {
        let key = (family.to_lowercase(), bold, italic);
        if let Some(found) = self.caches().defaults.get(&key) {
            return found.clone();
        }
        let mut result = None;
        for (name, kind) in default_substitutes(family, &self.font_table) {
            let found = match self.catalogue.lookup(&name, bold, italic) {
                Some(at) => Some((at, name.clone())),
                None => self.named(&name, bold, italic).map(|(at, s)| (at, s.substitute)),
            };
            if let Some((at, used)) = found {
                let class = (kind == SubstitutionKind::Generic).then(|| face_class(family, &self.font_table));
                result = Some((
                    at,
                    Substitution {
                        family: family.to_owned(),
                        substitute: used,
                        same_metrics: false,
                        kind,
                        class,
                    },
                ));
                break;
            }
        }
        self.caches().defaults.insert(key, result.clone());
        result
    }

    /// The advance of `ch` in the face Word lays `family` out with, and that face's units
    /// per em; where the face lacks the character (and fallbacks are on), the first face of
    /// Word's fallbacks that has it, else the face's own missing glyph. `None` where the
    /// family resolves to no face, or nothing answers.
    pub fn advance(&self, family: &str, bold: bool, italic: bool, ch: char) -> Option<GlyphAdvance> {
        let face = self.resolve(family, bold, italic).face?;
        if let Some(advance) = face.advance(ch) {
            return Some(GlyphAdvance {
                advance,
                units_per_em: face.units_per_em(),
                fallback: None,
            });
        }
        if !self.options.fallbacks {
            return None;
        }
        let key = ((family.to_owned(), bold, italic), ch);
        if let Some(found) = self.caches().glyphs.get(&key) {
            return found.clone();
        }
        let found = self.glyph_fallback(&face, family, bold, italic, ch);
        self.caches().glyphs.insert(key, found.clone());
        found
    }

    fn glyph_fallback(
        &self,
        face: &LayoutFace,
        family: &str,
        bold: bool,
        italic: bool,
        ch: char,
    ) -> Option<GlyphAdvance> {
        let mut names: Vec<String> = vec!["Times New Roman".to_owned()];
        names.extend(
            default_substitutes(family, &self.font_table)
                .into_iter()
                .map(|(n, _)| n),
        );
        for (_, faces) in GENERIC_FALLBACKS {
            names.extend(faces.iter().map(|n| (*n).to_owned()));
        }
        let mut seen: Vec<String> = Vec::new();
        for name in names {
            if seen.contains(&name) {
                continue;
            }
            seen.push(name.clone());
            let found = match self.catalogue.lookup(&name, bold, italic) {
                Some(at) => Some((at, name)),
                None => self.named(&name, bold, italic).map(|(at, s)| (at, s.substitute)),
            };
            let Some((at, used)) = found else { continue };
            let Some(other) = self.load(at) else { continue };
            if let Some(advance) = other.advance(ch) {
                return Some(GlyphAdvance {
                    advance,
                    units_per_em: other.units_per_em(),
                    fallback: Some(used),
                });
            }
        }
        match face {
            LayoutFace::Sfnt(own) => own.glyph_advance(0).map(|advance| GlyphAdvance {
                advance,
                units_per_em: own.units_per_em(),
                fallback: Some(".notdef".to_owned()),
            }),
            LayoutFace::Recorded(_) => None,
        }
    }

    /// The legacy kern adjustment between two characters in `family`'s face, font units;
    /// `None` where the family resolves to no face.
    pub fn kern(&self, family: &str, bold: bool, italic: bool, left: char, right: char) -> Option<i16> {
        self.resolve(family, bold, italic)
            .face
            .map(|face| face.kern(left, right))
    }

    /// The line and script metrics of `family`'s face, if it resolves to one.
    pub fn metrics(&self, family: &str, bold: bool, italic: bool) -> Option<FaceMetrics> {
        self.resolve(family, bold, italic).face.and_then(|face| face.metrics())
    }

    /// The faces of a class Word falls back to, for a report.
    pub fn class_fallbacks(class: FaceClass) -> &'static [&'static str] {
        generic_fallbacks(class)
    }
}

//! Owns: the index of every installed face by the family name a document gives it and the
//! style it asks for, built from a [`FontStore`]'s folders in the order Word searches them
//! (docx2svg's `installed_index`): the files of each folder sorted, first found winning --
//! except the families Word lays out from its own bundle although macOS has them
//! ([`WORD_PREFER_BUNDLE`]), whose bundle copy wins. And the catalogue's part of the fonts
//! epoch: a digest of the folders, the files and their stamps. And the installed faces
//! read so far ([`Catalogue::load`]): read once per catalogue and shared by every clone of
//! it, so the documents laid out with one machine's fonts hold one copy of each face
//! (lane L4b).
//!
//! Must not: substitute (that is [`crate::word`]), or read a whole file to index it.

use crate::scan::scan_file;
use crate::sfnt::Face;
use crate::store::FontStore;
use ooxml_base::{Digest, DigestKey, Digester};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// The families (lowercased) Word lays out from its bundle's copy although an earlier
/// folder has one (ooxml-common's `office.WORD.prefer_bundle`): the bullet line of a
/// measured résumé is SymbolMT's 1.2251 em, not the system Symbol's 1.0.
pub const WORD_PREFER_BUNDLE: [&str; 1] = ["symbol"];

/// The file suffixes a folder's font files have (any case).
const FONT_SUFFIXES: [&str; 3] = [".ttf", ".otf", ".ttc"];

/// The key the fonts epoch is digested under: the epoch names fonts the machine has, not
/// content an author writes.
pub(crate) const EPOCH_KEY: DigestKey = DigestKey::new(0x6f6f_786d_6c2d_666f, 0x6e74_7365_706f_6368);

/// What kind of folder a font folder is: only Word's bundle is treated apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Location {
    /// The operating system's, the user's, the cloud cache's or an application's folder.
    Folder,
    /// Word's own bundle (`Microsoft Word.app/Contents/Resources/DFonts`).
    WordBundle,
}

/// One folder searched for faces.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Folder {
    /// What kind of folder.
    pub location: Location,
    /// Its path, as the store names it.
    pub path: String,
}

/// Where an indexed face is: its file (an index into [`Catalogue::files`]) and its number
/// in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FaceLoc {
    /// The file.
    pub file: usize,
    /// The face's number in the file.
    pub number: usize,
}

type Key = (String, bool, bool);

/// The installed faces read so far, shared by a catalogue's clones.
type Loaded = Mutex<HashMap<FaceLoc, Option<Arc<Face>>>>;

/// Every installed face by `(family lowercased, bold, italic)`.
#[derive(Debug, Clone, Default)]
pub struct Catalogue {
    files: Vec<String>,
    index: HashMap<Key, FaceLoc>,
    epoch: Digest,
    loaded: Arc<Loaded>,
}

impl Catalogue {
    /// The catalogue of `folders`, searched in order.
    pub fn scan(store: &dyn FontStore, folders: &[Folder]) -> Catalogue {
        let mut out = Catalogue::default();
        let mut file_ids: HashMap<String, usize> = HashMap::new();
        let mut digest = Digester::new(EPOCH_KEY);
        for folder in folders {
            digest.update(folder.path.as_bytes());
            digest.update(&[0, u8::from(folder.location == Location::WordBundle)]);
            let Some(mut names) = store.entries(&folder.path) else {
                continue;
            };
            names.retain(|name| FONT_SUFFIXES.iter().any(|s| name.to_lowercase().ends_with(s)));
            names.sort();
            for name in names {
                let path = store.join(&folder.path, &name);
                let stamp = store.stamp(&path).unwrap_or_default();
                digest.update(path.as_bytes());
                digest.update(&stamp.modified_ns.to_le_bytes());
                digest.update(&stamp.size.to_le_bytes());
                let file = match file_ids.get(&path) {
                    Some(&known) => known,
                    None => {
                        out.files.push(path.clone());
                        file_ids.insert(path.clone(), out.files.len() - 1);
                        out.files.len() - 1
                    }
                };
                let faces = store.open(&path).map(|f| scan_file(f.as_ref())).unwrap_or_default();
                out.add(folder.location, file, &faces);
            }
        }
        out.epoch = digest.finish();
        out
    }

    fn add(&mut self, location: Location, file: usize, faces: &[crate::scan::ScannedFace]) {
        for face in faces {
            let Some((bold, italic, families)) = &face.named else {
                continue;
            };
            let at = FaceLoc {
                file,
                number: face.number,
            };
            for family in families {
                let key = (family.to_lowercase(), *bold, *italic);
                if location == Location::WordBundle && WORD_PREFER_BUNDLE.contains(&key.0.as_str()) {
                    self.index.insert(key, at);
                } else {
                    self.index.entry(key).or_insert(at);
                }
            }
        }
    }

    /// The face for `family` in a style: the style asked for, else bold without italic,
    /// else regular (docx2svg's `_lookup`).
    pub fn lookup(&self, family: &str, bold: bool, italic: bool) -> Option<FaceLoc> {
        lookup(&self.index, family, bold, italic)
    }

    /// Every face the index holds, each once, in no order.
    pub fn faces(&self) -> Vec<FaceLoc> {
        let mut out: Vec<FaceLoc> = self.index.values().copied().collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The face at `at`, read through `store` the first time any clone of this catalogue
    /// asks; `None` where it cannot be read.
    pub(crate) fn load(&self, store: &dyn FontStore, at: FaceLoc) -> Option<Arc<Face>> {
        let loaded = || self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(found) = loaded().get(&at) {
            return found.clone();
        }
        let path = self.file(at.file)?;
        let face = store
            .read(path)
            .ok()
            .and_then(|data| Face::new(data, at.number, path).ok())
            .map(Arc::new);
        loaded().entry(at).or_insert(face).clone()
    }

    /// The path of an indexed file.
    pub fn file(&self, file: usize) -> Option<&str> {
        self.files.get(file).map(String::as_str)
    }

    /// Every indexed file's path, in the order first found.
    pub fn files(&self) -> &[String] {
        &self.files
    }

    /// A digest of the folders, their files and the files' stamps.
    pub fn epoch(&self) -> Digest {
        self.epoch
    }
}

/// `(key, bold, italic)`, else `(key, bold, false)`, else `(key, false, false)`.
pub(crate) fn lookup<V: Clone>(index: &HashMap<Key, V>, family: &str, bold: bool, italic: bool) -> Option<V> {
    let key = family.to_lowercase();
    [(bold, italic), (bold, false), (false, false)]
        .into_iter()
        .find_map(|(b, i)| index.get(&(key.clone(), b, i)).cloned())
}

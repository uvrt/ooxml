//! Owns: the font database a drawing's text is drawn with ([`FontSet`]) and how one is put
//! together ([`FontLoader`]): the system's fonts, files and folders read where they are
//! installed, faces handed over as bytes (a document's embedded faces, or files read through
//! `ooxml-fonts`' [`FontStore`] port), and the faces the generic families name
//! ([`Families`]).
//!
//! resvg chooses a face from its database the way Python's rasteriser does only if the
//! database holds the same faces in the same order: of two faces that answer to one name,
//! weight, stretch and style, the first loaded draws both. resvg-py 0.5 loads the system's
//! fonts first, then the files it is given, then the folders
//! ([`FontLoader::resvg_py`]). A folder is walked by fontdb itself, in the order the
//! operating system lists it, as resvg-py's is.
//!
//! Must not: choose a face (resvg's database does), rewrite a font, or be rebuilt per
//! drawing: a set is built once and shared ([`FontSet`] is a cheap clone of an `Arc`).

use ooxml_fonts::{FontError, FontStore};
use std::path::Path;
use std::sync::Arc;

/// The faces the default and generic families name: usvg's `font_family` and fontdb's
/// `serif`, `sans-serif`, `cursive`, `fantasy` and `monospace` targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Families {
    /// The family a text that names none is drawn in (usvg's `font_family`).
    pub default: String,
    /// `serif`.
    pub serif: String,
    /// `sans-serif`.
    pub sans_serif: String,
    /// `cursive`.
    pub cursive: String,
    /// `fantasy`.
    pub fantasy: String,
    /// `monospace`.
    pub monospace: String,
}

impl Families {
    /// resvg-py 0.5's defaults on this platform (`svg_to_bytes` where the caller names
    /// none): Microsoft's faces on macOS and Windows, the Liberation faces on Linux, the
    /// generic names elsewhere.
    pub fn platform_default() -> Families {
        let [default, serif, sans_serif, cursive, fantasy, monospace] =
            if cfg!(any(target_os = "macos", target_os = "windows")) {
                [
                    "Times New Roman",
                    "Times New Roman",
                    "Arial",
                    "Comic Sans MS",
                    "Impact",
                    "Courier New",
                ]
            } else if cfg!(target_os = "linux") {
                [
                    "Liberation Serif",
                    "Liberation Serif",
                    "Liberation Sans",
                    "Comic Neue",
                    "Anton",
                    "Liberation Mono",
                ]
            } else {
                ["serif", "serif", "sans-serif", "cursive", "fantasy", "monospace"]
            };
        Families {
            default: default.to_owned(),
            serif: serif.to_owned(),
            sans_serif: sans_serif.to_owned(),
            cursive: cursive.to_owned(),
            fantasy: fantasy.to_owned(),
            monospace: monospace.to_owned(),
        }
    }
}

impl Default for Families {
    fn default() -> Families {
        Families::platform_default()
    }
}

/// The faces drawings are drawn with: built once, shared by every drawing after.
#[derive(Debug, Clone)]
pub struct FontSet {
    database: Arc<fontdb::Database>,
    default_family: String,
}

impl FontSet {
    /// A set with no faces: a drawing's text draws nothing (an SVG without text needs none).
    pub fn empty() -> FontSet {
        FontLoader::new(Families::platform_default()).build()
    }

    /// How many faces the set holds.
    pub fn len(&self) -> usize {
        self.database.len()
    }

    /// Whether the set holds no face.
    pub fn is_empty(&self) -> bool {
        self.database.is_empty()
    }

    /// The family a text that names none is drawn in.
    pub fn default_family(&self) -> &str {
        &self.default_family
    }

    pub(crate) fn database(&self) -> Arc<fontdb::Database> {
        Arc::clone(&self.database)
    }
}

/// Puts a [`FontSet`] together, face by face, in the order faces are to be preferred.
#[derive(Debug, Clone)]
pub struct FontLoader {
    database: fontdb::Database,
    families: Families,
}

impl FontLoader {
    /// No faces yet; the generic families pointed at `families`.
    pub fn new(families: Families) -> FontLoader {
        FontLoader {
            database: fontdb::Database::new(),
            families,
        }
    }

    /// `set`'s faces to start from, in its order (a set of the system's fonts, read once,
    /// extended for each document); the generic families pointed at `families`.
    pub fn extending(set: &FontSet, families: Families) -> FontLoader {
        FontLoader {
            database: (*set.database).clone(),
            families,
        }
    }

    /// The set resvg-py 0.5 draws with when handed `files`, `folders` and (unless
    /// `system` is false) the system's fonts: the system's first, then each file, then
    /// each folder. A file that cannot be read is left out, as resvg-py leaves it out.
    pub fn resvg_py(system: Option<&FontSet>, files: &[&Path], folders: &[&Path], families: Families) -> FontSet {
        let mut loader = match system {
            Some(set) => FontLoader::extending(set, families),
            None => FontLoader::new(families),
        };
        for file in files {
            // resvg-py logs the failure and goes on.
            let _ = loader.file(file);
        }
        for folder in folders {
            loader.folder(folder);
        }
        loader.build()
    }

    /// The system's fonts (fontdb's `load_system_fonts`: the platform's font folders, or
    /// fontconfig's on Linux), read where they are installed.
    pub fn system(&mut self) -> &mut FontLoader {
        self.database.load_system_fonts();
        self
    }

    /// Every face of the file at `path`, read where it is.
    ///
    /// # Errors
    /// The file cannot be opened.
    pub fn file(&mut self, path: &Path) -> std::io::Result<&mut FontLoader> {
        self.database.load_font_file(path)?;
        Ok(self)
    }

    /// Every face of every font file under `folder`, walked by fontdb in the order the
    /// operating system lists each folder (resvg-py's order); what cannot be read is left
    /// out.
    pub fn folder(&mut self, folder: &Path) -> &mut FontLoader {
        self.database.load_fonts_dir(folder);
        self
    }

    /// Every face of a font file's bytes: an embedded face, de-obfuscated.
    pub fn bytes(&mut self, data: Arc<[u8]>) -> &mut FontLoader {
        self.database.load_font_source(fontdb::Source::Binary(Arc::new(data)));
        self
    }

    /// Every face of the file `path` names in `store`, read through the port.
    ///
    /// # Errors
    /// The store cannot read the file.
    pub fn from_store(&mut self, store: &dyn FontStore, path: &str) -> Result<&mut FontLoader, FontError> {
        let data = store.read(path)?;
        Ok(self.bytes(data))
    }

    /// The set, ready to draw with.
    pub fn build(mut self) -> FontSet {
        let Families {
            default,
            serif,
            sans_serif,
            cursive,
            fantasy,
            monospace,
        } = self.families;
        self.database.set_serif_family(serif);
        self.database.set_sans_serif_family(sans_serif);
        self.database.set_cursive_family(cursive);
        self.database.set_fantasy_family(fantasy);
        self.database.set_monospace_family(monospace);
        FontSet {
            database: Arc::new(self.database),
            default_family: default,
        }
    }
}

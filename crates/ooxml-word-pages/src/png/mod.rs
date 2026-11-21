//! Owns: a page's PNG as docx2svg's `svg_to_png` draws it with resvg -- the faces handed
//! over ([`PageRaster`]: the files and folders, the system's fonts or not, the face the
//! generic `sans-serif` names, read once), a metafile picture's SVG written inline on the
//! way in ([`inline`]), the faces a text loses to a face of another family under one name
//! found again without the rival's files (docx2svg's drawing plan, its passes drawn as one:
//! `ooxml_fonts::plan`), and the drawing itself (`ooxml-raster`); a document's raster made
//! from one with the system's fonts and folders, handed the faces its pages draw
//! ([`faces`], [`PageRaster::for_faces`]).
//!
//! Must not: lay out or paint (the page's SVG is given), or change the SVG handed out.

pub mod faces;
pub mod inline;
pub mod sized;

use ooxml_fonts::plan::{text_queries, FacePool};
use ooxml_fonts::FontStore;
pub use ooxml_raster::{Fit, Rgba};

use faces::{RasterFaces, RasterFile};
use ooxml_raster::{rasterise, Families, FontLoader, FontSet, RasterError, RasterOptions, Withheld};
use std::path::Path;
use std::sync::Arc;

/// The faces a document's pages are drawn with, read once.
pub struct PageRaster {
    fonts: FontSet,
    pool: Option<FacePool>,
    /// The system's fonts it was handed, for a document's raster made from it.
    system: Option<FontSet>,
    /// The folders it was handed, for a document's raster made from it.
    folders: Vec<String>,
}

impl std::fmt::Debug for PageRaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageRaster")
            .field("faces", &self.fonts.len())
            .finish_non_exhaustive()
    }
}

impl PageRaster {
    /// The faces resvg-py is handed: `system` (the system's fonts, read once by the caller,
    /// or none), then `files`, then `folders`; `sans_serif` the face the generic family
    /// names (the platform's default where `None`). `store` reads the handed files for the
    /// plan.
    pub fn new(
        store: &dyn FontStore,
        system: Option<&FontSet>,
        (files, folders): (&[String], &[String]),
        sans_serif: Option<&str>,
    ) -> PageRaster {
        let paths: Vec<&Path> = files.iter().map(Path::new).collect();
        let dirs: Vec<&Path> = folders.iter().map(Path::new).collect();
        let fonts = FontLoader::resvg_py(system, &paths, &dirs, families(sans_serif));
        let pool = FacePool::read(store, &fonts.handed_files());
        PageRaster {
            fonts,
            pool,
            system: system.cloned(),
            folders: folders.to_vec(),
        }
    }

    /// The raster that draws a document's pages as docx2svg's `convert_docx_to_png` draws
    /// them, made from this one's system fonts and folders: `faces`' files, each read where
    /// it is (an embedded face from its bytes), after the system's fonts only where a face
    /// is missing, then the folders; `sans-serif` the face `faces` names. `store` reads the
    /// files for the drawing plan.
    #[must_use]
    pub fn for_faces(&self, store: &dyn FontStore, faces: &RasterFaces) -> PageRaster {
        let families = families(faces.sans_serif.as_deref());
        let mut loader = match self.system.as_ref().filter(|_| !faces.skip_system()) {
            Some(set) => FontLoader::extending(set, families),
            None => FontLoader::new(families),
        };
        for file in &faces.files {
            match file {
                // resvg-py logs a file it cannot read and goes on.
                RasterFile::Path(path) => {
                    let _ = loader.file(Path::new(path));
                }
                RasterFile::Embedded(bytes) => {
                    loader.bytes(Arc::clone(bytes));
                }
            }
        }
        for folder in &self.folders {
            loader.folder(Path::new(folder));
        }
        let fonts = loader.build();
        let pool = FacePool::read(store, &fonts.handed_files());
        PageRaster {
            fonts,
            pool,
            system: self.system.clone(),
            folders: self.folders.clone(),
        }
    }

    /// `svg` drawn at `fit` over `background`, as PNG bytes.
    ///
    /// # Errors
    /// The SVG cannot be read or drawn at that size.
    pub fn png(&self, svg: &str, fit: Fit, background: Option<Rgba>) -> Result<Vec<u8>, RasterError> {
        let svg = inline::inline_metafile_images(svg);
        let withheld = self.withheld(&svg);
        let options = RasterOptions {
            fit,
            background,
            withheld,
        };
        rasterise(&svg, &self.fonts, &options)?.png()
    }

    /// The files each text's face is chosen without (`drawing_plan`), where any.
    fn withheld(&self, svg: &str) -> Option<Withheld> {
        let pool = self.pool.as_ref()?;
        if !svg.contains("<text") {
            return None;
        }
        let queries = text_queries(svg);
        let plan = pool.plan(&queries)?;
        let mut out = Withheld::default();
        for (value, found) in &queries {
            let pass = plan
                .passes
                .iter()
                .skip(1)
                .position(|(values, _)| values.contains(value))
                .map_or(0, |k| k + 1);
            let files = plan.passes.get(pass).map(|(_, w)| w.clone()).unwrap_or_default();
            for query in found {
                out.texts.insert(query.clone(), files.clone());
            }
        }
        Some(out)
    }
}

/// The generic families, `sans-serif` pointed at `sans_serif` (the platform's default where
/// `None` or empty).
fn families(sans_serif: Option<&str>) -> Families {
    let mut families = Families::platform_default();
    if let Some(face) = sans_serif.filter(|f| !f.is_empty()) {
        face.clone_into(&mut families.sans_serif);
    }
    families
}

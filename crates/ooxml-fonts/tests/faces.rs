//! The reader and the lookup over the open faces the repository ships (pptx2svg-fonts'
//! files), against the values docx2svg's `fonts.Face` reads from the same files.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_fonts::{
    Face, FileStamp, Folder, FontError, FontFile, FontStore, Grade, Location, WordFontOptions, WordFonts,
};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

const FILES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../python/pptx2svg/packages/pptx2svg-fonts/src/pptx2svg_fonts/files"
);

struct Disk;
struct DiskFile(std::fs::File);

impl FontFile for DiskFile {
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, FontError> {
        let mut file = &self.0;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| FontError::Store(e.to_string()))?;
        let mut out = Vec::new();
        file.take(len as u64)
            .read_to_end(&mut out)
            .map_err(|e| FontError::Store(e.to_string()))?;
        Ok(out)
    }
}

impl FontStore for Disk {
    fn entries(&self, folder: &str) -> Option<Vec<String>> {
        let listing = std::fs::read_dir(folder).ok()?;
        Some(listing.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect())
    }
    fn open(&self, path: &str) -> Result<Box<dyn FontFile + '_>, FontError> {
        Ok(Box::new(DiskFile(
            std::fs::File::open(path).map_err(|e| FontError::Store(e.to_string()))?,
        )))
    }
    fn read(&self, path: &str) -> Result<Arc<[u8]>, FontError> {
        std::fs::read(path)
            .map(Arc::from)
            .map_err(|e| FontError::Store(e.to_string()))
    }
    fn stamp(&self, path: &str) -> Option<FileStamp> {
        let meta = std::fs::metadata(path).ok()?;
        Some(FileStamp {
            modified_ns: 0,
            size: meta.len(),
        })
    }
}

fn face(name: &str) -> Face {
    let data = std::fs::read(Path::new(FILES).join(name)).unwrap();
    Face::new(Arc::from(data), 0, name).unwrap()
}

#[test]
fn carlito_reads_as_docx2svg_reads_it() {
    let carlito = face("Carlito-Regular.ttf");
    assert_eq!(carlito.units_per_em(), 2048);
    assert_eq!(carlito.style().unwrap(), (false, false));
    assert_eq!(carlito.families().into_iter().collect::<Vec<_>>(), ["Carlito"]);
    let m = carlito.metrics().unwrap();
    assert_eq!((m.ascent, m.descent, m.line_gap), (1950, 550, 0));
    assert_eq!((m.superscript_size, m.subscript_size), (Some(1331), Some(1331)));
    assert_eq!((m.superscript_offset, m.subscript_offset), (Some(976), Some(286)));
    let d = carlito.decorations().unwrap();
    assert_eq!((d.strikeout_position, d.strikeout_size), (Some(512), Some(134)));
    assert_eq!((d.underline_position, d.underline_thickness), (Some(-103), Some(194)));
    let advances: Vec<Option<u16>> = "AVa \u{e7}\u{20ac}\u{4e00}"
        .chars()
        .map(|c| carlito.advance(c))
        .collect();
    assert_eq!(
        advances,
        [
            Some(1185),
            Some(1162),
            Some(981),
            Some(463),
            Some(866),
            Some(1038),
            None
        ]
    );
    assert_eq!(carlito.glyph('A'), Some(3));
    assert_eq!(carlito.kern('A', 'V'), 0);
}

#[test]
fn tinos_bold_and_caladea_italic() {
    let tinos = face("Tinos-Bold.ttf");
    assert_eq!(tinos.style().unwrap(), (true, false));
    let m = tinos.metrics().unwrap();
    assert_eq!((m.ascent, m.descent, m.line_gap), (1825, 443, 87));
    assert_eq!(tinos.advance('A'), Some(1479));
    let caladea = face("Caladea-Italic.ttf");
    assert_eq!(caladea.units_per_em(), 1000);
    assert_eq!(caladea.style().unwrap(), (false, true));
    assert_eq!(caladea.advance('\u{20ac}'), Some(601));
}

#[test]
fn word_finds_styles_substitutes_and_reports_them() {
    let folders = [Folder {
        location: Location::Folder,
        path: FILES.to_owned(),
    }];
    let fonts = WordFonts::new(
        Arc::new(Disk),
        &folders,
        &[],
        Default::default(),
        WordFontOptions::default(),
    );
    let found = fonts.resolve("Carlito", true, true);
    assert_eq!(found.grade, Grade::Installed);
    assert!(
        matches!(found.face, Some(ooxml_fonts::LayoutFace::Sfnt(ref f)) if f.source().ends_with("Carlito-BoldItalic.ttf"))
    );
    // Calibri is not there: Carlito, its measured compatible substitute, is.
    match fonts.resolve("Calibri", false, false).grade {
        Grade::Substituted(s) => assert!(s.same_metrics && s.substitute == "Carlito"),
        other => panic!("{other:?}"),
    }
    // Aptos has no named substitute: the default one, approximate.
    match fonts.resolve("Aptos", false, false).grade {
        Grade::Substituted(s) => assert!(!s.same_metrics && s.substitute == "Carlito"),
        other => panic!("{other:?}"),
    }
    assert_eq!(fonts.resolve("Symbol", false, false).grade, Grade::Recorded);
    let advance = fonts.advance("Carlito", false, false, 'A').unwrap();
    assert_eq!(
        (advance.advance, advance.units_per_em, advance.fallback),
        (1185, 2048, None)
    );
    // A character Carlito lacks comes from the first fallback that has it.
    let ideograph = fonts.advance("Carlito", false, false, '\u{4e00}').unwrap();
    assert!(ideograph.fallback.is_some());
    let none = WordFonts::new(
        Arc::new(Disk),
        &folders,
        &[],
        Default::default(),
        WordFontOptions {
            substitutes: Vec::new(),
            recorded_faces: false,
            fallbacks: false,
        },
    );
    assert_eq!(none.resolve("Calibri", false, false).grade, Grade::Missing);
    assert_ne!(none.epoch(), fonts.epoch());
}

#[test]
fn a_character_map_of_groups_laid_over_each_other_is_refused_quickly() {
    // The fuzzer's `glyph_ink` timeout, made up: a `cmap` whose format 12 subtable states
    // 200 groups each claiming every code point (220 million insertions, were they made).
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/cmap-groups-laid-over.ttf"
    ))
    .unwrap();
    let started = std::time::Instant::now();
    let face = Face::new(Arc::from(data), 0, "made up").unwrap();
    assert_eq!(face.glyph('A'), None);
    for slant in [0.0, 0.34] {
        assert_eq!(face.ink("AVATAR Tyre", slant), None);
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

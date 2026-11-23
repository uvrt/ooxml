//! Owns: a package's pictures as the page SVG reads them ([`PackagePictures`], the
//! `Pictures` port): a picture part's bytes and media type by its relationship, a
//! metafile drawn as SVG by `ooxml-metafile` (as docx2svg draws one with ooxml-common).
//!
//! It keeps the package without the parts the Word model reads into the version (the
//! main document part, its headers, footers and notes): those are the version's text,
//! held in its model, and a version kept for its pictures must not hold a second copy of
//! them as parsed trees (lane L4b). The relationship parts, the content types and every
//! other part stay, shared with the package they came from.
//!
//! A metafile is drawn once for as long as its engine's memo keeps it ([`PictureMemo`]):
//! keyed by its part's content ([`ooxml_opc::PartIdentity`]), so the versions sharing the
//! part, and every page showing it (a logo in a header), take the same drawing.
//!
//! Must not: decode, scale or re-encode a picture.

use ooxml_derive::{CacheBudget, Memo, Weigh};
use ooxml_opc::{Package, PartIdentity};
use ooxml_word_layout::paint::svg::Pictures;
use std::sync::{Arc, Mutex, PoisonError};

const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
/// The relationship types of the parts the model reads as the version's text.
const TEXT_PARTS: [&str; 5] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments",
];

/// A metafile part drawn: its SVG, or `None` where it draws nothing (the part is shown as
/// it is).
#[derive(Clone)]
struct Drawn(Option<Arc<[u8]>>);

impl Weigh for Drawn {
    fn weight(&self) -> usize {
        self.0.as_ref().map_or(0, |svg| svg.len())
    }
}

/// The metafiles an engine drew, by their parts' content, within a budget.
pub struct PictureMemo(Mutex<Memo<PartIdentity, Drawn>>);

impl PictureMemo {
    /// An empty memo reserving from `budget`.
    pub fn new(budget: Arc<dyn CacheBudget>) -> PictureMemo {
        PictureMemo(Mutex::new(Memo::new(budget)))
    }

    /// The drawing of the metafile `part` (whose bytes `bytes` reads), made where not kept.
    fn drawn(&self, part: &PartIdentity, bytes: impl FnOnce() -> Option<Arc<[u8]>>) -> Option<Drawn> {
        let held = self.0.lock().unwrap_or_else(PoisonError::into_inner).get(part);
        if held.is_some() {
            return held;
        }
        let made = Drawn(draw(&bytes()?));
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(part.clone(), made.clone());
        Some(made)
    }
}

impl std::fmt::Debug for PictureMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PictureMemo").finish_non_exhaustive()
    }
}

/// A metafile's bytes drawn as SVG, where it draws anything.
fn draw(bytes: &[u8]) -> Option<Arc<[u8]>> {
    let drawing = ooxml_metafile::draw(bytes, &ooxml_metafile::Options::default())?;
    (drawing.drawn > 0).then(|| Arc::from(drawing.svg()))
}

/// The pictures of one version's package.
#[derive(Debug, Clone)]
pub struct PackagePictures {
    package: Package,
    main: String,
    /// Where its metafiles' drawings are kept (`None`: each drawn when read).
    drawn: Option<Arc<PictureMemo>>,
}

impl PackagePictures {
    /// The pictures of `package`; `None` where it has no main document part. The main
    /// part and the story parts it relates to are left out (the module's notes).
    pub fn new(package: &Package) -> Option<PackagePictures> {
        let main = package
            .relationships("")
            .ok()?
            .iter()
            .find(|r| r.rel_type == OFFICE)
            .and_then(|r| r.target_part.clone())?;
        let mut text: Vec<String> = package
            .relationships(&main)
            .map(|rels| {
                rels.iter()
                    .filter(|r| !r.external && TEXT_PARTS.contains(&r.rel_type.as_str()))
                    .filter_map(|r| r.target_part.clone())
                    .collect()
            })
            .unwrap_or_default();
        text.push(main.clone());
        let mut kept = package.clone();
        for name in &text {
            if kept.has_part(name) {
                kept = kept.without(name).ok()?;
            }
        }
        Some(PackagePictures {
            package: kept,
            main,
            drawn: None,
        })
    }

    /// These pictures, their metafiles' drawings kept in `memo`.
    #[must_use]
    pub fn with_memo(mut self, memo: Arc<PictureMemo>) -> PackagePictures {
        self.drawn = Some(memo);
        self
    }

    /// The package, without the parts the model reads as the version's text.
    pub fn package(&self) -> &Package {
        &self.package
    }

    /// The main document part's name.
    pub fn main_part(&self) -> &str {
        &self.main
    }
}

impl Pictures for PackagePictures {
    fn picture(&self, relationship: &str, part: Option<&str>) -> Option<(String, Arc<[u8]>)> {
        let source = part.unwrap_or(&self.main);
        let rels = self.package.relationships(source).ok()?;
        let target = rels.get(relationship)?.target_part.clone()?;
        let read = || self.package.read(&target).ok().flatten();
        let mime = self.package.content_types().ok()?.of(&target)?.to_owned();
        if !matches!(mime.as_str(), "image/x-emf" | "image/emf" | "image/x-wmf" | "image/wmf") {
            return Some((mime, read()?));
        }
        let svg = match (&self.drawn, self.package.part(&target)) {
            (Some(memo), Some(part)) => memo.drawn(&part.identity(), read)?.0,
            _ => draw(&read()?),
        };
        match svg {
            Some(svg) => Some(("image/svg+xml".to_owned(), svg)),
            None => Some((mime, read()?)),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use ooxml_zip::{Method, NewEntry, Writer};

    /// An EMF: a red solid brush selected, a rectangle, the end (ooxml-metafile's players test).
    const EMF: &str = "0100000058000000000000000000000063000000310000000000000000000000560a00002b05000020454d460000\
0100a800000005000000010000000000000000000000000000006000000060000000190000001900000027000000180000000100000000000000ff0\
0000000000000250000000c000000010000002b000000180000000a0000000a0000003c000000280000000e0000001400000000000000000000001\
4000000";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0))
            .collect()
    }

    struct Unlimited;

    impl CacheBudget for Unlimited {
        fn reserve(&self, _bytes: usize) -> bool {
            true
        }
        fn release(&self, _bytes: usize) {}
        fn used(&self) -> usize {
            0
        }
    }

    fn package() -> Package {
        let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="emf" ContentType="image/x-emf"/><Default Extension="png" ContentType="image/png"/></Types>"#;
        let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="urn:image" Target="media/a.emf"/><Relationship Id="rId2" Type="urn:image" Target="media/b.png"/><Relationship Id="rId3" Type="urn:image" Target="media/c.emf"/></Relationships>"#;
        let doc = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body/></w:document>"#;
        let emf = hex(EMF);
        let mut w = Writer::new();
        for (name, data) in [
            ("[Content_Types].xml", types.as_bytes()),
            ("_rels/.rels", root.as_bytes()),
            ("word/document.xml", doc.as_bytes()),
            ("word/_rels/document.xml.rels", rels.as_bytes()),
            ("word/media/a.emf", emf.as_slice()),
            ("word/media/b.png", b"\x89PNG\r\n".as_slice()),
            ("word/media/c.emf", b"not a metafile".as_slice()),
        ] {
            let entry = NewEntry {
                name: name.into(),
                method: Method::Deflated,
                dos_time: 0x6000,
                dos_date: 0x5a21,
                external_attr: 0,
                create_system: 0,
            };
            w.add(&entry, data).unwrap();
        }
        let bytes = w.finish(b"").unwrap();
        Package::open(Arc::from(bytes), &ooxml_base::Limits::default()).unwrap()
    }

    #[test]
    fn a_metafile_is_drawn_once_for_every_version_sharing_its_part_as_drawn_each_time() {
        let first = package();
        let memo = Arc::new(PictureMemo::new(Arc::new(Unlimited)));
        let plain = PackagePictures::new(&first).unwrap();
        let kept = PackagePictures::new(&first).unwrap().with_memo(Arc::clone(&memo));
        for id in ["rId1", "rId2", "rId3", "rId9"] {
            assert_eq!(kept.picture(id, None), plain.picture(id, None), "{id}");
        }
        let (mime, svg) = kept.picture("rId1", None).unwrap();
        assert_eq!(mime, "image/svg+xml");
        // what draws nothing is shown as it is
        assert_eq!(kept.picture("rId3", None).unwrap().0, "image/x-emf");
        // a later version sharing the part takes the very drawing; one whose part is new draws it
        let later = first.with_bytes("word/media/b.png", Arc::from(&b"\x89PNG"[..]));
        let again = PackagePictures::new(&later).unwrap().with_memo(Arc::clone(&memo));
        assert!(Arc::ptr_eq(&again.picture("rId1", None).unwrap().1, &svg));
        let other = first.with_bytes("word/media/a.emf", Arc::from(hex(EMF)));
        let drawn = PackagePictures::new(&other).unwrap().with_memo(memo);
        let (_, made) = drawn.picture("rId1", None).unwrap();
        assert!(!Arc::ptr_eq(&made, &svg));
        assert_eq!(made, svg);
    }
}

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
//! Must not: decode, scale or re-encode a picture.

use ooxml_opc::Package;
use ooxml_word_layout::paint::svg::Pictures;
use std::sync::Arc;

const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
/// The relationship types of the parts the model reads as the version's text.
const TEXT_PARTS: [&str; 5] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments",
];

/// The pictures of one version's package.
#[derive(Debug, Clone)]
pub struct PackagePictures {
    package: Package,
    main: String,
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
        Some(PackagePictures { package: kept, main })
    }

    /// The package, without the parts the model reads as the version's text.
    pub fn package(&self) -> &Package {
        &self.package
    }
}

impl Pictures for PackagePictures {
    fn picture(&self, relationship: &str, part: Option<&str>) -> Option<(String, Arc<[u8]>)> {
        let source = part.unwrap_or(&self.main);
        let rels = self.package.relationships(source).ok()?;
        let target = rels.get(relationship)?.target_part.clone()?;
        let bytes = self.package.read(&target).ok()??;
        let mime = self.package.content_types().ok()?.of(&target)?.to_owned();
        if matches!(mime.as_str(), "image/x-emf" | "image/emf" | "image/x-wmf" | "image/wmf") {
            if let Some(drawing) = ooxml_metafile::draw(&bytes, &ooxml_metafile::Options::default()) {
                if drawing.drawn > 0 {
                    return Some(("image/svg+xml".to_owned(), Arc::from(drawing.svg())));
                }
            }
        }
        Some((mime, bytes))
    }
}

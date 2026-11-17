//! Owns: a package's pictures as the page SVG reads them ([`PackagePictures`], the
//! `Pictures` port): a picture part's bytes and media type by its relationship, a
//! metafile drawn as SVG by `ooxml-metafile` (as docx2svg draws one with ooxml-common).
//!
//! Must not: decode, scale or re-encode a picture.

use ooxml_opc::Package;
use ooxml_word_layout::paint::svg::Pictures;
use std::sync::Arc;

const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

/// The pictures of one version's package.
#[derive(Debug, Clone)]
pub struct PackagePictures {
    package: Package,
    main: String,
}

impl PackagePictures {
    /// The pictures of `package`; `None` where it has no main document part.
    pub fn new(package: Package) -> Option<PackagePictures> {
        let main = package
            .relationships("")
            .ok()?
            .iter()
            .find(|r| r.rel_type == OFFICE)
            .and_then(|r| r.target_part.clone())?;
        Some(PackagePictures { package, main })
    }

    /// The package.
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

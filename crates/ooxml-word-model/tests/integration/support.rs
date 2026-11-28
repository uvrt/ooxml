//! Made-up Word packages for the model's tests: a body, and optionally styles, numbering and
//! settings, written as a package in memory.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use ooxml_base::{DigestKey, Limits};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::{Model, WordDocument, WordPackage};
use ooxml_zip::{Method, NewEntry, Writer};
use std::sync::Arc;

/// The WordprocessingML namespace declaration every made-up part uses.
pub const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;

/// A budget without a limit.
pub struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

/// The parts of a made-up document.
#[derive(Default, Clone)]
pub struct Parts {
    /// `w:body`'s content.
    pub body: String,
    /// `w:styles`' content, if any.
    pub styles: Option<String>,
    /// `w:numbering`'s content, if any.
    pub numbering: Option<String>,
    /// `w:settings`' content, if any.
    pub settings: Option<String>,
}

fn rel(id: &str, kind: &str, target: &str) -> String {
    format!(
        r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#
    )
}

/// The package's bytes.
pub fn package_bytes(parts: &Parts) -> Vec<u8> {
    let mut w = Writer::new();
    let entry = |name: &str| NewEntry {
        name: name.into(),
        method: Method::Deflated,
        dos_time: 0x6000,
        dos_date: 0x5a21,
        external_attr: 0,
        create_system: 0,
    };
    let mut rels = String::new();
    let mut files: Vec<(String, String)> = vec![(
        "word/document.xml".into(),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W}><w:body>{}</w:body></w:document>"#,
            parts.body
        ),
    )];
    for (k, (part, kind, root)) in [
        (&parts.styles, "styles", "styles"),
        (&parts.numbering, "numbering", "numbering"),
        (&parts.settings, "settings", "settings"),
    ]
    .into_iter()
    .enumerate()
    {
        if let Some(content) = part {
            rels.push_str(&rel(&format!("rId{}", k + 1), kind, &format!("{kind}.xml")));
            files.push((
                format!("word/{kind}.xml"),
                format!(r#"<w:{root} {W}>{content}</w:{root}>"#),
            ));
        }
    }
    files.push((
        "word/_rels/document.xml.rels".into(),
        format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#),
    ));
    files.push((
        "_rels/.rels".into(),
        format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}</Relationships>"#,
            rel("rId1", "officeDocument", "word/document.xml")
        ),
    ));
    files.push((
        "[Content_Types].xml".into(),
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.into(),
    ));
    for (name, data) in files {
        w.add(&entry(&name), data.as_bytes()).unwrap();
    }
    w.finish(b"").unwrap()
}

/// The package as opened: every block without an id given one ([`WordPackage::open`]).
pub fn package(parts: &Parts) -> WordPackage {
    WordPackage::open(raw_package(parts)).0
}

/// The package as its bytes have it.
pub fn raw_package(parts: &Parts) -> Package {
    Package::open(Arc::from(package_bytes(parts)), &Limits::default()).unwrap()
}

/// A document's model and its versions as read so far: each version's parts share every
/// subtree they have alike with the version before's (as an edit's do), and its reading is
/// made from the one before (the document's one chain of readings).
pub struct Versions {
    model: Model,
    last: Option<(Package, WordPackage, Arc<Docx>)>,
}

impl std::ops::Deref for Versions {
    type Target = Model;

    fn deref(&self) -> &Model {
        &self.model
    }
}

impl std::ops::DerefMut for Versions {
    fn deref_mut(&mut self) -> &mut Model {
        &mut self.model
    }
}

impl Versions {
    /// The version `parts` make, read after the one opened before and projected.
    pub fn open(&mut self, parts: &Parts) -> WordDocument {
        let mut raw = raw_package(parts);
        if let Some((before, _, _)) = &self.last {
            for name in raw.part_names() {
                let (Ok(Some(old)), Ok(Some(new))) = (before.tree(&name), raw.tree(&name)) else {
                    continue;
                };
                raw = raw.with_new_tree(&name, ooxml_xml::shared_with(Some(old), new));
            }
        }
        let (package, _) = WordPackage::open_after(raw.clone(), self.last.as_ref().map(|(_, p, _)| p));
        let reading = Arc::new(match &self.last {
            Some((_, _, before)) => Docx::open_after(&package, before).unwrap(),
            None => Docx::open(&package, Arc::new(Unlimited)).unwrap(),
        });
        let document = self.model.project(&reading, None).unwrap();
        self.last = Some((raw, package, reading));
        document
    }
}

/// A fresh model of a new document.
pub fn model() -> Versions {
    Versions {
        model: Model::new(Arc::new(Unlimited), DigestKey::new(1, 2)),
        last: None,
    }
}

/// The document opened in a fresh model.
pub fn open(parts: &Parts) -> (Versions, WordDocument) {
    let mut m = model();
    let d = m.open(parts);
    (m, d)
}

/// `body` with every paragraph stamped with a paraId of its own (as every version an edit
/// makes is): its blocks' keys follow from content.
pub fn stamped(body: &str) -> String {
    let mut out = String::new();
    for (k, piece) in body.split("<w:p>").enumerate() {
        if k > 0 {
            out.push_str(&format!(r#"<w:p w14:paraId="{:08X}">"#, 0x1000_0000 + k));
        }
        out.push_str(piece);
    }
    out
}

/// A paragraph with one run of `text`, and `ppr` / `rpr` as its properties.
pub fn p(ppr: &str, rpr: &str, text: &str) -> String {
    format!(r#"<w:p><w:pPr>{ppr}</w:pPr><w:r><w:rPr>{rpr}</w:rPr><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#)
}

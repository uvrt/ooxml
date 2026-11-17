//! Made-up documents and a made-up face for the layout's tests: a package written in
//! memory (a body and optional settings), opened by the Word model, and laid out with
//! one face (every character 1024 units of 2048; ascent 1854, descent 434, gap 67).
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use ooxml_base::{Digest, DigestKey, Limits, Locale};
use ooxml_derive::CacheBudget;
use ooxml_fonts::FaceMetrics;
use ooxml_opc::Package;
use ooxml_text::{Advance, FaceKey, Glyphs, ScriptMetrics};
use ooxml_word_layout::table::NoTables;
use ooxml_word_layout::{Faces, WordLayout};
use ooxml_word_model::{Model, WordDocument};
use ooxml_zip::{Method, NewEntry, Writer};
use std::sync::Arc;

/// The namespaces every made-up part declares.
pub const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

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

/// One face for everything.
pub struct Mono;

impl Glyphs for Mono {
    fn advance(&self, _: &FaceKey, _: char) -> Option<Advance> {
        Some(Advance {
            advance: 1024,
            units_per_em: 2048,
        })
    }
    fn kern(&self, _: &FaceKey, _: char, _: char) -> Option<i32> {
        Some(0)
    }
    fn script_metrics(&self, _: &FaceKey) -> Option<ScriptMetrics> {
        None
    }
    fn epoch(&self) -> Digest {
        Digest::default()
    }
}

impl Faces for Mono {
    fn metrics(&self, _: &FaceKey) -> Option<FaceMetrics> {
        Some(FaceMetrics {
            units_per_em: 2048,
            ascent: 1854,
            descent: 434,
            line_gap: 67,
            superscript_size: None,
            subscript_size: None,
            superscript_offset: None,
            subscript_offset: None,
        })
    }
}

fn rel(id: &str, kind: &str, target: &str) -> String {
    format!(
        r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#
    )
}

/// A package of `body` (`w:body`'s content) and `settings` (`w:settings`' content).
pub fn package(body: &str, settings: Option<&str>) -> Package {
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
    let mut files = vec![(
        "word/document.xml".to_owned(),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W}><w:body>{body}</w:body></w:document>"#
        ),
    )];
    if let Some(content) = settings {
        rels.push_str(&rel("rId1", "settings", "settings.xml"));
        files.push((
            "word/settings.xml".into(),
            format!(r#"<w:settings {W}>{content}</w:settings>"#),
        ));
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
    Package::open(Arc::from(w.finish(b"").unwrap()), &Limits::default()).unwrap()
}

/// A fresh model.
pub fn model() -> Model {
    Model::new(Arc::new(Unlimited), DigestKey::new(1, 2))
}

/// A layout engine over [`Mono`].
pub fn engine() -> WordLayout {
    let budget: Arc<dyn CacheBudget> = Arc::new(Unlimited);
    WordLayout::new(
        Arc::new(Mono),
        Arc::new(NoTables),
        Locale::default(),
        &budget,
        DigestKey::new(3, 4),
    )
}

/// Open `body` in `model`.
pub fn open(model: &mut Model, body: &str, settings: Option<&str>) -> WordDocument {
    model.open(&package(body, settings)).unwrap()
}

/// A paragraph: `ppr` its properties, `content` its runs' XML.
pub fn para(ppr: &str, content: &str) -> String {
    format!(r#"<w:p><w:pPr>{ppr}</w:pPr>{content}</w:p>"#)
}

/// A run of `text`.
pub fn run(text: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{text}</w:t></w:r>"#)
}

/// A complex field: its instruction and cached result.
pub fn field(instruction: &str, cached: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {instruction} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        run(cached)
    )
}

/// A section's properties: a small page (2880 x 2880 twips, margins 360) of `columns`
/// columns, started by `kind`.
pub fn sect(kind: &str, columns: u32) -> String {
    format!(
        r#"<w:sectPr><w:type w:val="{kind}"/><w:pgSz w:w="2880" w:h="2880"/><w:pgMar w:top="360" w:right="360" w:bottom="360" w:left="360" w:header="180" w:footer="180" w:gutter="0"/><w:cols w:num="{columns}" w:space="120"/></w:sectPr>"#
    )
}

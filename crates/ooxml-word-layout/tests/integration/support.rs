//! Made-up documents and a made-up face for the layout's tests: a package written in
//! memory (a body and optional settings), opened by the Word model, and laid out with
//! one face (every character 1024 units of 2048; ascent 1854, descent 434, gap 67).
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use ooxml_base::{Digest, DigestKey, Limits, Locale};
use ooxml_derive::CacheBudget;
use ooxml_fonts::{Decorations, FaceMetrics};
use ooxml_opc::Package;
use ooxml_text::{Advance, FaceKey, Glyphs, ScriptMetrics};
use ooxml_word_layout::tables::WordTables;
use ooxml_word_layout::{Faces, WordLayout};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::{Model, WordDocument};
use ooxml_zip::{Method, NewEntry, Writer};
use std::sync::Arc;

/// The namespaces every made-up part declares.
pub const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing""#;

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
    fn decorations(&self, _: &FaceKey) -> Option<Decorations> {
        None
    }
}

fn rel(id: &str, kind: &str, target: &str) -> String {
    format!(
        r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#
    )
}

/// A package of `body` (`w:body`'s content) and `settings` (`w:settings`' content).
pub fn package(body: &str, settings: Option<&str>) -> Package {
    package_with(body, settings, &[])
}

/// A part of the main document beside its body: its relationship id, type and name, and
/// its whole XML.
pub struct Part<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub name: &'a str,
    pub xml: String,
}

/// A package of `body`, `settings` and `parts`.
pub fn package_with(body: &str, settings: Option<&str>, parts: &[Part<'_>]) -> Package {
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
    for part in parts {
        rels.push_str(&rel(part.id, part.kind, part.name));
        files.push((format!("word/{}", part.name), part.xml.clone()));
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

/// A document's model and its chain of readings: each version opened is read from the one
/// opened before, as a document's versions are.
pub struct Versions {
    model: Model,
    last: Option<Arc<Docx>>,
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
    /// `package` read after the version opened before (else first) and projected.
    pub fn open(&mut self, package: Package) -> WordDocument {
        let package = ooxml_word_model::WordPackage::open(package).0;
        let reading = Arc::new(match &self.last {
            Some(before) => Docx::open_after(&package, before).unwrap(),
            None => Docx::open(&package, Arc::new(Unlimited)).unwrap(),
        });
        self.last = Some(Arc::clone(&reading));
        self.model.project(&reading, None).unwrap()
    }
}

/// A fresh model of a new document.
pub fn model() -> Versions {
    Versions {
        model: Model::new(Arc::new(Unlimited), DigestKey::new(1, 2)),
        last: None,
    }
}

/// A layout engine over [`Mono`].
pub fn engine() -> WordLayout {
    let budget: Arc<dyn CacheBudget> = Arc::new(Unlimited);
    WordLayout::new(
        Arc::new(Mono),
        Arc::new(WordTables),
        Locale::default(),
        &budget,
        DigestKey::new(3, 4),
    )
}

/// Open `body` in `model`.
pub fn open(model: &mut Versions, body: &str, settings: Option<&str>) -> WordDocument {
    model.open(package(body, settings))
}

/// Open `body` with `parts` in `model`.
pub fn open_with(model: &mut Versions, body: &str, settings: Option<&str>, parts: &[Part<'_>]) -> WordDocument {
    model.open(package_with(body, settings, parts))
}

/// A floating drawing `cx` x `cy` twips at the column's and paragraph's offsets `x`, `y`
/// twips, wrapped by `wrap` (`wrapSquare`, `wrapTopAndBottom`...).
pub fn anchor(wrap: &str, x: i64, y: i64, cx: i64, cy: i64) -> String {
    let side = if wrap == "wrapSquare" {
        r#" wrapText="bothSides""#
    } else {
        ""
    };
    format!(
        r#"<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="1" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>{}</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>{}</wp:posOffset></wp:positionV><wp:extent cx="{}" cy="{}"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:{wrap}{side}/><wp:docPr id="1" name="d"/></wp:anchor></w:drawing></w:r>"#,
        x * 635,
        y * 635,
        cx * 635,
        cy * 635
    )
}

/// A notes part (`footnote` or `endnote`): its separators and `notes` (id, text).
pub fn notes_part(kind: &str, notes: &[(u32, String)]) -> String {
    let mut xml = format!(
        r#"<w:{kind}s {W}><w:{kind} w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:{kind}><w:{kind} w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:{kind}>"#
    );
    for (id, text) in notes {
        xml.push_str(&format!(
            r#"<w:{kind} w:id="{id}"><w:p><w:r><w:{kind}Ref/></w:r>{}</w:p></w:{kind}>"#,
            run(text)
        ));
    }
    xml.push_str(&format!("</w:{kind}s>"));
    xml
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

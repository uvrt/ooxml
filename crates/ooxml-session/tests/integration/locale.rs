//! The locale a call reads its documents under (lane P5q): a table of contents' `\t` styles
//! split on the locale's list separator and found in its language -- the call's locale
//! first, then the session's where the host set one, then the document's own language
//! (`w:themeFontLang`), as docx-agent's `_list_separator` and `_field_language` read them;
//! the pages from a page engine over the repository's test fonts.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, NoFonts, NoParts, Unlimited};
use ooxml_base::context::Timestamp;
use ooxml_base::{Clock, Context, Limits};
use ooxml_derive::CacheBudget;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_numfmt::{Locale, EN_US};
use ooxml_opc::Package;
use ooxml_session::{CallContext, DocId, DocKind, Session, SessionConfig};
use ooxml_tools_word::EnginePages;
use ooxml_word_pages::{DiskFonts, EngineBudgets, EngineOptions, FontSetup, PageEngine};
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

struct Fixed;
impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp {
            seconds: 1_795_000_000,
            nanos: 0,
        }
    }
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

/// A memo budget that grants every reservation (the page engine's).
struct Memos;

impl CacheBudget for Memos {
    fn reserve(&self, _: usize) -> bool {
        true
    }
    fn release(&self, _: usize) {}
    fn used(&self) -> usize {
        0
    }
}

/// The repository's test fonts, scanned.
pub(crate) fn test_fonts() -> Arc<FontSetup> {
    let folder = Folder {
        location: Location::Folder,
        path: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../python/pptx2svg/packages/pptx2svg-fonts/src/pptx2svg_fonts/files"
        )
        .to_owned(),
    };
    let options = WordFontOptions {
        substitutes: Vec::new(),
        recorded_faces: true,
        fallbacks: true,
    };
    Arc::new(FontSetup::new(Arc::new(DiskFonts), &[folder], options))
}

/// The page engine the host lays the documents out with (the repository's test fonts).
fn layout() -> Arc<EnginePages> {
    let setup = test_fonts();
    let budgets = EngineBudgets {
        held: Arc::new(Memos),
        paint: Arc::new(Memos),
    };
    let engine = PageEngine::new(setup, EngineOptions::default(), budgets);
    Arc::new(EnginePages::of(engine))
}

pub(crate) fn session_with(config: SessionConfig) -> Session {
    let context = Context {
        fonts: Arc::new(NoFonts),
        parts: Arc::new(NoParts),
        clock: Arc::new(Fixed),
        budget: Arc::new(Unlimited),
        cancel: Arc::new(Never),
        limits: Limits::default(),
        locale: config.locale.as_ref().map(ooxml_base::Locale::from).unwrap_or_default(),
    };
    Session::new(config, context)
}

fn session(locale: Option<Locale>) -> Session {
    session_with(SessionConfig {
        locale,
        ..SessionConfig::default()
    })
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A document with two headings (Word's built-in `heading 1` and `heading 2`, ids `Kop1`
/// and `Kop2` as Dutch Word saves them) and a table of contents naming them in Dutch,
/// split on `;`; its settings' `w:themeFontLang` is `theme` (none where `None`).
pub(crate) fn docx(theme: Option<&str>) -> Arc<[u8]> {
    let ns = format!(r#"xmlns:w="{W}" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#);
    let heading = |id: &str, style: &str, text: &str| {
        format!(
            r#"<w:p w14:paraId="{id}" w14:textId="77777777"><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        )
    };
    let toc = r#"<w:p w14:paraId="10000003" w14:textId="77777777"><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \h \t "Kop 1;1;Kop 2;2" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>old</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
    let body = format!(
        "{toc}{}{}",
        heading("10000001", "Kop1", "Eerste"),
        heading("10000002", "Kop2", "Tweede")
    );
    let document = format!("<w:document {ns}><w:body>{body}</w:body></w:document>");
    let style = |id: &str, name: &str, level: u8| {
        format!(
            r#"<w:style w:type="paragraph" w:styleId="{id}"><w:name w:val="{name}"/><w:pPr><w:outlineLvl w:val="{level}"/></w:pPr></w:style>"#
        )
    };
    let styles = format!(
        r#"<w:styles xmlns:w="{W}"><w:style w:type="paragraph" w:default="1" w:styleId="Standaard"><w:name w:val="Normal"/></w:style>{}{}</w:styles>"#,
        style("Kop1", "heading 1", 0),
        style("Kop2", "heading 2", 1)
    );
    let lang = theme.map_or_else(String::new, |t| format!(r#"<w:themeFontLang w:val="{t}"/>"#));
    let settings = format!(r#"<w:settings xmlns:w="{W}">{lang}</w:settings>"#);
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/styles" Target="styles.xml"/><Relationship Id="rId2" Type="{REL}/settings" Target="settings.xml"/></Relationships>"#
    );
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/></Types>"#;
    let mut w = Writer::new();
    for (name, data) in [
        ("[Content_Types].xml", types.to_owned()),
        ("_rels/.rels", rels.to_owned()),
        ("word/document.xml", document),
        ("word/_rels/document.xml.rels", doc_rels),
        ("word/styles.xml", styles),
        ("word/settings.xml", settings),
    ] {
        let entry = NewEntry {
            name: name.into(),
            method: Method::Deflated,
            dos_time: 0,
            dos_date: 33,
            external_attr: 0,
            create_system: 3,
        };
        w.add(&entry, data.as_bytes()).unwrap();
    }
    Arc::from(w.finish(b"").unwrap())
}

/// Dutch with its list separator, `;`.
fn dutch() -> Locale {
    Locale::new(',', '.', "dmy", "nl-NL", ';').unwrap()
}

/// Dutch named by its language only: ooxml-common's `Locale(language="nl-NL")`, whose list
/// separator is the default `,`.
fn dutch_language() -> Locale {
    Locale {
        language: "nl-NL".into(),
        ..EN_US
    }
}

/// The headings the table of contents lists once its fields are updated under `call` (the
/// call's locale) in `session`: those of the two whose text is in it as well as in the body.
fn listed(session: &Session, theme: Option<&str>, call: Option<Locale>) -> Vec<&'static str> {
    let doc: DocId = session.open(docx(theme), DocKind::Docx, "d.docx").unwrap();
    let context = CallContext {
        layout: Some(layout()),
        locale: call,
        ..CallContext::default()
    };
    let reply = session.call(
        "word_fields",
        json!({"doc": doc.to_string(), "action": "update"}),
        &context,
    );
    let reply = serde_json::Value::Object(reply.to_json());
    assert!(reply.get("error").is_none(), "{reply}");
    let saved = session.save(&doc).unwrap();
    let package = Package::open(Arc::from(saved), &Limits::default()).unwrap();
    let body = String::from_utf8(package.read("word/document.xml").unwrap().unwrap().to_vec()).unwrap();
    ["Eerste", "Tweede"]
        .into_iter()
        .filter(|text| body.matches(text).count() > 1)
        .collect()
}

#[test]
fn a_document_without_a_locale_is_read_in_its_own_language() {
    // themeFontLang nl-NL: Dutch names, split on Dutch Word's `;`
    assert_eq!(listed(&session(None), Some("nl-NL"), None), ["Eerste", "Tweede"]);
    // no language: English names split on `,`, so neither Dutch name is found
    assert!(listed(&session(None), None, None).is_empty());
}

#[test]
fn the_calls_locale_comes_first() {
    // the call's list separator, set: `;` splits the names, found in Dutch
    assert_eq!(listed(&session(None), None, Some(dutch())), ["Eerste", "Tweede"]);
    // the call's locale names Dutch alone: its list separator is `,`, whatever the
    // document's language says
    assert!(listed(&session(None), Some("nl-NL"), Some(dutch_language())).is_empty());
    // before the session's
    assert_eq!(
        listed(&session(Some(dutch_language())), None, Some(dutch())),
        ["Eerste", "Tweede"]
    );
}

#[test]
fn a_session_locale_the_host_set_applies_where_a_call_names_none() {
    assert_eq!(listed(&session(Some(dutch())), None, None), ["Eerste", "Tweede"]);
    assert!(listed(&session(Some(dutch_language())), Some("nl-NL"), None).is_empty());
}

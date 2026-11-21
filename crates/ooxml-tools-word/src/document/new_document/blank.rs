//! Owns: a new blank document's package, written the way Word writes a new blank document,
//! as docx-agent's `edit/blank.py` (`build`) writes it: one empty paragraph and the section
//! (the page size, margins, header and footer distances, columns, the document grid); the
//! styles, settings, web settings, font table and Office theme Word 16.106 made a new
//! document of ([`word_new`], generated); the core and extended properties. What depends on
//! the call: the page (a size Word names, portrait or landscape), the text language (the
//! document defaults' and the theme font language), the number separators (the user's
//! regional settings), the page's defaults (the Normal template's: [`PageDefaults::METRIC`],
//! measured; [`PageDefaults::US`] for a Letter page), the title, author and date.
//!
//! Must not: open the document (the host opens the bytes in its session) or read a template
//! (a new document from one is not made here yet).

use super::word_new;
use ooxml_word_model::ids::IdPool;
use ooxml_zip::{Method, NewEntry, Writer};

/// What a new document's page takes from Word's Normal template (lengths in twips).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageDefaults {
    /// Every margin.
    pub margin: u32,
    /// The header's distance from the edge.
    pub header: u32,
    /// The footer's.
    pub footer: u32,
    /// The default tab stop.
    pub tab_stop: u32,
    /// The space between columns.
    pub column_space: u32,
    /// `w:hyphenationZone`; `None` writes none (Word's default, 0.25 inch).
    pub hyphenation_zone: Option<u32>,
}

impl PageDefaults {
    /// Measured: Word 16.106 for Mac, Dutch interface, metric units.
    pub const METRIC: PageDefaults = PageDefaults {
        margin: 1417,
        header: 708,
        footer: 708,
        tab_stop: 708,
        column_space: 708,
        hyphenation_zone: Some(425),
    };
    /// English (United States) Word's defaults: Letter, 1 inch, half an inch.
    pub const US: PageDefaults = PageDefaults {
        margin: 1440,
        header: 720,
        footer: 720,
        tab_stop: 720,
        column_space: 720,
        hyphenation_zone: None,
    };
}

/// The page sizes a new document names, twips (width, height), portrait.
pub const PAGE_SIZES: [(&str, u32, u32); 5] = [
    ("A4", 11906, 16838),
    ("Letter", 12240, 15840),
    ("Legal", 12240, 20160),
    ("A5", 8391, 11906),
    ("A3", 16838, 23811),
];

/// A new blank document as asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blank {
    /// The page's size, one of [`PAGE_SIZES`].
    pub page: String,
    /// Landscape: the longer side across.
    pub landscape: bool,
    /// The text language (`w:lang`), a BCP 47 tag.
    pub language: String,
    /// The decimal symbol and the list separator (the regional settings').
    pub separators: (String, String),
    /// The title property, where given.
    pub title: Option<String>,
    /// The author (creator and last modifier), where given.
    pub author: Option<String>,
    /// When it was made, W3CDTF to the second (`2026-11-21T09:00:00Z`).
    pub created: String,
}

/// Text escaped as lxml escapes it in content.
fn text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\r', "&#13;")
}

/// An attribute value escaped as `edit/blank.py` escapes it.
fn attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
const CT: &str = "application/vnd.openxmlformats-officedocument.";

impl Blank {
    /// The page's width and height, twips.
    ///
    /// # Errors
    /// A page size not in [`PAGE_SIZES`].
    pub fn size(&self) -> Result<(u32, u32), String> {
        let (_, width, height) = PAGE_SIZES
            .iter()
            .find(|(name, _, _)| *name == self.page)
            .ok_or_else(|| format!("unknown page size {:?}", self.page))?;
        Ok(if self.landscape {
            ((*width).max(*height), (*width).min(*height))
        } else {
            (*width, *height)
        })
    }

    fn defaults(&self) -> PageDefaults {
        if self.page == "Letter" {
            PageDefaults::US
        } else {
            PageDefaults::METRIC
        }
    }

    fn document(&self) -> Result<String, String> {
        let (width, height) = self.size()?;
        let mut ids = IdPool::default().generate("word/document.xml", 2).into_iter();
        let (para, text_id) = (ids.next().unwrap_or_default(), ids.next().unwrap_or_default());
        let page = self.defaults();
        let m = page.margin;
        let orient = if self.landscape { " w:orient=\"landscape\"" } else { "" };
        Ok(format!(
            "{DECL}{root}<w:body><w:p w14:paraId=\"{para}\" w14:textId=\"{text_id}\"/>\
             <w:sectPr><w:pgSz w:w=\"{width}\" w:h=\"{height}\"{orient}/>\
             <w:pgMar w:top=\"{m}\" w:right=\"{m}\" w:bottom=\"{m}\" w:left=\"{m}\" w:header=\"{h}\" \
             w:footer=\"{f}\" w:gutter=\"0\"/><w:cols w:space=\"{c}\"/>\
             <w:docGrid w:linePitch=\"360\"/></w:sectPr></w:body></w:document>",
            root = word_new::DOCUMENT_ROOT,
            h = page.header,
            f = page.footer,
            c = page.column_space,
        ))
    }

    fn settings(&self) -> String {
        let page = self.defaults();
        let zone = page
            .hyphenation_zone
            .map(|z| format!("<w:hyphenationZone w:val=\"{z}\"/>"))
            .unwrap_or_default();
        format!(
            "{head}<w:defaultTabStop w:val=\"{tab}\"/>{zone}{compat}<w:themeFontLang w:val=\"{lang}{after}{decimal}{between}{list}{tail}",
            head = word_new::SETTINGS_HEAD,
            tab = page.tab_stop,
            compat = word_new::SETTINGS_COMPAT,
            lang = attribute(&self.language),
            after = word_new::SETTINGS_AFTER_LANGUAGE,
            decimal = attribute(&self.separators.0),
            between = word_new::SETTINGS_AFTER_DECIMAL,
            list = attribute(&self.separators.1),
            tail = word_new::SETTINGS_TAIL,
        )
    }

    fn core(&self) -> String {
        let element = |tag: &str, value: &str| {
            // lxml writes text set to "" as <x></x>
            format!("<{tag}>{}</{tag}>", text(value))
        };
        let mut out = format!(
            "{DECL}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
             xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" \
             xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">"
        );
        out.push_str(&element("dc:title", self.title.as_deref().unwrap_or("")));
        out.push_str(&element("dc:subject", ""));
        if let Some(author) = &self.author {
            out.push_str(&element("dc:creator", author));
        }
        out.push_str(&element("cp:keywords", ""));
        out.push_str(&element("dc:description", ""));
        if let Some(author) = &self.author {
            out.push_str(&element("cp:lastModifiedBy", author));
        }
        out.push_str(&element("cp:revision", "1"));
        for tag in ["dcterms:created", "dcterms:modified"] {
            out.push_str(&format!(
                "<{tag} xsi:type=\"dcterms:W3CDTF\">{}</{tag}>",
                text(&self.created)
            ));
        }
        out.push_str("</cp:coreProperties>");
        out
    }

    /// The package's bytes, as `edit/blank.py`'s `build` zips them: every entry deflated,
    /// dated 1980-01-01, made on Unix.
    ///
    /// # Errors
    /// A page size not in [`PAGE_SIZES`]; the archive not written.
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        let mut w = Writer::new();
        for (name, data) in self.parts()? {
            let entry = NewEntry {
                name: name.to_owned(),
                method: Method::Deflated,
                dos_time: 0,
                dos_date: 33,
                external_attr: 0,
                create_system: 3,
            };
            w.add(&entry, data.as_bytes()).map_err(|e| e.to_string())?;
        }
        w.finish(b"").map_err(|e| e.to_string())
    }

    /// The parts, in the order `build` writes them.
    fn parts(&self) -> Result<Vec<(&'static str, String)>, String> {
        let core_rel = "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
        Ok(vec![
            ("[Content_Types].xml", content_types()),
            (
                "_rels/.rels",
                relationships(&[
                    ("rId3", format!("{R}extended-properties"), "docProps/app.xml"),
                    ("rId2", core_rel.to_owned(), "docProps/core.xml"),
                    ("rId1", format!("{R}officeDocument"), "word/document.xml"),
                ]),
            ),
            ("word/document.xml", self.document()?),
            (
                "word/_rels/document.xml.rels",
                relationships(&[
                    ("rId3", format!("{R}webSettings"), "webSettings.xml"),
                    ("rId2", format!("{R}settings"), "settings.xml"),
                    ("rId1", format!("{R}styles"), "styles.xml"),
                    ("rId5", format!("{R}theme"), "theme/theme1.xml"),
                    ("rId4", format!("{R}fontTable"), "fontTable.xml"),
                ]),
            ),
            ("word/theme/theme1.xml", word_new::THEME.to_owned()),
            ("word/settings.xml", self.settings()),
            (
                "word/styles.xml",
                format!(
                    "{}{}{}",
                    word_new::STYLES_BEFORE_LANGUAGE,
                    attribute(&self.language),
                    word_new::STYLES_AFTER_LANGUAGE
                ),
            ),
            ("word/webSettings.xml", word_new::WEB_SETTINGS.to_owned()),
            ("word/fontTable.xml", word_new::FONT_TABLE.to_owned()),
            ("docProps/core.xml", self.core()),
            ("docProps/app.xml", word_new::APP.to_owned()),
        ])
    }
}

/// A relationships part of `items` (`(id, type, target)`).
fn relationships(items: &[(&str, String, &str)]) -> String {
    let mut out =
        format!("{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
    for (id, kind, target) in items {
        out.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\"/>"
        ));
    }
    out + "</Relationships>"
}

/// `[Content_Types].xml`: the two defaults, an override for each part.
fn content_types() -> String {
    let overrides = [
        ("word/document.xml", format!("{CT}wordprocessingml.document.main+xml")),
        ("word/styles.xml", format!("{CT}wordprocessingml.styles+xml")),
        ("word/settings.xml", format!("{CT}wordprocessingml.settings+xml")),
        ("word/webSettings.xml", format!("{CT}wordprocessingml.webSettings+xml")),
        ("word/fontTable.xml", format!("{CT}wordprocessingml.fontTable+xml")),
        ("word/theme/theme1.xml", format!("{CT}theme+xml")),
        (
            "docProps/core.xml",
            "application/vnd.openxmlformats-package.core-properties+xml".to_owned(),
        ),
        ("docProps/app.xml", format!("{CT}extended-properties+xml")),
    ];
    let mut types = format!(
        "{DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>"
    );
    for (part, kind) in &overrides {
        types.push_str(&format!("<Override PartName=\"/{part}\" ContentType=\"{kind}\"/>"));
    }
    types.push_str("</Types>");
    types
}

#[cfg(test)]
mod tests {
    use super::Blank;
    use ooxml_base::Limits;
    use ooxml_opc::Package;
    use std::sync::Arc;

    fn blank(page: &str, landscape: bool) -> Blank {
        Blank {
            page: page.to_owned(),
            landscape,
            language: "nl-NL".to_owned(),
            separators: (",".to_owned(), ";".to_owned()),
            title: Some("R&D".to_owned()),
            author: None,
            created: "2026-11-21T09:30:15Z".to_owned(),
        }
    }

    #[test]
    fn a_landscape_letter_page_with_the_us_defaults() {
        let made = blank("Letter", true);
        assert_eq!(made.size().unwrap(), (15840, 12240));
        let package = Package::open(Arc::from(made.bytes().unwrap()), &Limits::default()).unwrap();
        let read = |name: &str| String::from_utf8(package.read(name).unwrap().unwrap().to_vec()).unwrap();
        let body = read("word/document.xml");
        assert!(
            body.contains(r#"<w:pgSz w:w="15840" w:h="12240" w:orient="landscape"/>"#),
            "{body}"
        );
        assert!(body.contains(r#"w:top="1440""#));
        let settings = read("word/settings.xml");
        assert!(settings.contains(r#"<w:defaultTabStop w:val="720"/><w:characterSpacingControl"#));
        assert!(settings.contains(r#"<w:themeFontLang w:val="nl-NL"/>"#));
        assert!(settings.contains(r#"<w:decimalSymbol w:val=","/><w:listSeparator w:val=";"/>"#));
        let core = read("docProps/core.xml");
        assert!(core.contains("<dc:title>R&amp;D</dc:title><dc:subject></dc:subject><cp:keywords>"));
        assert!(read("word/styles.xml").contains(r#"<w:lang w:val="nl-NL" w:eastAsia="en-US""#));
    }

    #[test]
    fn an_unknown_page_is_refused() {
        assert!(blank("B5", false).bytes().is_err());
    }
}

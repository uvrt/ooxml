//! An application's live view of a Word document on the session (lane LV2): every page and
//! its blocks first, then the pages an edit changed and those it added; a page's SVG of the
//! version a token names, tagged with that version's ids (a header's too), drawn after the
//! document moved on; tokens kept for the last four versions; the refusals and errors; views
//! read while another thread edits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{layout, Never, NoFonts, NoParts, Unlimited};
use crate::editing::{call, docx, session};
use ooxml_base::context::Timestamp;
use ooxml_base::{Clock, Context, Limits, Locale};
use ooxml_session::{CallContext, DocKind, Session, SessionConfig, TextMode, ViewError, VIEW_TOKENS};
use ooxml_tools_word::EnginePages;
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

/// A paragraph `id` reading `text`, starting a page where `page_break`.
fn paragraph(id: &str, text: &str, page_break: bool) -> String {
    let before = if page_break {
        "<w:pPr><w:pageBreakBefore/></w:pPr>"
    } else {
        ""
    };
    format!(r#"<w:p w14:paraId="{id}" w14:textId="77777777">{before}<w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

/// Two pages: `one` on the first, `two` on the second.
fn two_pages() -> String {
    paragraph("11111111", "one", false) + &paragraph("22222222", "two", true)
}

/// A session made a second after [`session`]'s.
fn restarted() -> Session {
    struct Later;
    impl Clock for Later {
        fn now(&self) -> Timestamp {
            Timestamp {
                seconds: 1_795_000_001,
                nanos: 0,
            }
        }
        fn monotonic(&self) -> Duration {
            Duration::ZERO
        }
    }
    let context = Context {
        fonts: Arc::new(NoFonts),
        parts: Arc::new(NoParts),
        clock: Arc::new(Later),
        budget: Arc::new(Unlimited),
        cancel: Arc::new(Never),
        limits: Limits::default(),
        locale: Locale::default(),
    };
    Session::new(SessionConfig::default(), context)
}

fn ctx() -> CallContext {
    CallContext::default()
}

fn opened(body: &str) -> (Session, ooxml_session::DocId, Arc<EnginePages>) {
    let s = session();
    let d = s.open(docx(body), DocKind::Docx, "d.docx").unwrap();
    (s, d, layout())
}

#[test]
fn a_view_lists_every_page_then_the_pages_an_edit_changed() {
    let (s, d, lay) = opened(&two_pages());
    let first = s.view(&d, None, &*lay, &ctx()).unwrap();
    assert_eq!(first.pages.len(), 2);
    assert_eq!(first.pages[0].blocks, ["p:11111111"]);
    assert_eq!(first.pages[1].blocks, ["p:22222222"]);
    assert!(first.pages[0].width > 500.0 && first.pages[0].height > first.pages[0].width);
    assert_eq!(first.changed, [1, 2]);
    assert!(!first.stale && first.note.is_none() && first.since.is_none());
    assert!(
        first.token.ends_with(&format!("-{d}-{}", first.version)),
        "{}",
        first.token
    );

    let again = s.view(&d, Some(&first.token), &*lay, &ctx()).unwrap();
    assert_eq!((again.token.as_str(), again.changed.len()), (first.token.as_str(), 0));

    let edited = call(
        &s,
        "word_set_text",
        json!({"doc": "d1", "target": "p:22222222", "text": "deux"}),
        &ctx(),
    );
    assert_eq!(edited["ok"], json!(true), "{edited}");
    let second = s.view(&d, Some(&first.token), &*lay, &ctx()).unwrap();
    assert_ne!(second.token, first.token);
    assert_eq!(second.changed, [2], "only the second page draws what changed");
    assert_eq!(second.since.as_deref(), Some(first.token.as_str()));

    // the first version's page, drawn after the document moved on, and the second's
    let old = s.page_svg(&d, 2, &first.token, &*lay, TextMode::Text, &ctx()).unwrap();
    let new = s.page_svg(&d, 2, &second.token, &*lay, TextMode::Text, &ctx()).unwrap();
    assert_ne!(old.svg, new.svg);
    assert!(old.svg.contains(r#"data-docx-agent-id="p:22222222""#), "{}", old.svg);
    let front = s.page_svg(&d, 1, &second.token, &*lay, TextMode::Text, &ctx()).unwrap();
    assert!(front.svg.contains(r#"data-docx-agent-id="p:11111111""#));
    assert!(!front.svg.contains("p:22222222"));
}

#[test]
fn a_page_added_is_changed_and_one_removed_is_not_listed() {
    let (s, d, lay) = opened(&paragraph("11111111", "one", false));
    let first = s.view(&d, None, &*lay, &ctx()).unwrap();
    assert_eq!(first.pages.len(), 1);
    let parts = |body: String| {
        let xml = format!("<w:document {NS}><w:body>{body}</w:body></w:document>");
        vec![("word/document.xml".to_owned(), Some(Arc::from(xml.as_bytes())))]
    };
    s.update(&d, parts(two_pages()), &ctx()).unwrap();
    let second = s.view(&d, Some(&first.token), &*lay, &ctx()).unwrap();
    assert_eq!(second.pages.len(), 2);
    assert_eq!(second.changed, [2]);
    s.update(&d, parts(paragraph("11111111", "one", false)), &ctx())
        .unwrap();
    let third = s.view(&d, Some(&second.token), &*lay, &ctx()).unwrap();
    assert_eq!((third.pages.len(), third.changed.len()), (1, 0));
}

#[test]
fn tokens_are_kept_for_the_last_four_versions() {
    let (s, d, lay) = opened(&two_pages());
    let mut tokens = vec![s.view(&d, None, &*lay, &ctx()).unwrap().token];
    for n in 0..VIEW_TOKENS {
        let text = format!("two {n}");
        let edited = call(
            &s,
            "word_set_text",
            json!({"doc": "d1", "target": "p:22222222", "text": text}),
            &ctx(),
        );
        assert_eq!(edited["ok"], json!(true), "{edited}");
        tokens.push(s.view(&d, None, &*lay, &ctx()).unwrap().token);
    }
    let gone = s.page_svg(&d, 1, &tokens[0], &*lay, TextMode::Text, &ctx());
    assert!(matches!(gone, Err(ViewError::UnknownToken { .. })), "{gone:?}");
    let stale = s.view(&d, Some(&tokens[0]), &*lay, &ctx()).unwrap();
    assert!(stale.stale);
    assert_eq!(stale.changed, [1, 2]);
    assert!(stale
        .note
        .as_deref()
        .unwrap()
        .contains("every page is listed as changed"));
    for token in &tokens[1..] {
        assert!(s.page_svg(&d, 2, token, &*lay, TextMode::Text, &ctx()).is_ok());
    }
}

#[test]
fn a_page_or_mode_it_does_not_draw_is_refused() {
    let (s, d, lay) = opened(&two_pages());
    let view = s.view(&d, None, &*lay, &ctx()).unwrap();
    let past = s.page_svg(&d, 3, &view.token, &*lay, TextMode::Text, &ctx());
    assert!(
        matches!(past, Err(ViewError::NoPage { page: 3, count: 2, .. })),
        "{past:?}"
    );
    let zero = s.page_svg(&d, 0, &view.token, &*lay, TextMode::Text, &ctx());
    assert!(matches!(zero, Err(ViewError::NoPage { .. })));
    let Err(ViewError::NotDrawn(outlines)) = s.page_svg(&d, 1, &view.token, &*lay, TextMode::Outlines, &ctx()) else {
        panic!("outlines are refused")
    };
    assert_eq!(outlines.code().as_str(), "refused");
    assert_eq!(outlines.field(), Some("text"));
    assert!(matches!(TextMode::parse("paths"), Err(ViewError::Mode(_))));
    // another session's token (one from before a restart, made at another moment) names
    // nothing here
    let other = restarted();
    let o = other.open(docx(&two_pages()), DocKind::Docx, "d.docx").unwrap();
    let theirs = other.view(&o, None, &*layout(), &ctx()).unwrap();
    let unknown = s.page_svg(&d, 1, &theirs.token, &*lay, TextMode::Text, &ctx());
    assert!(matches!(unknown, Err(ViewError::UnknownToken { .. })), "{unknown:?}");
    let not_open = s.view(&ooxml_session::DocId::parse("d9").unwrap(), None, &*lay, &ctx());
    assert!(matches!(not_open, Err(ViewError::Session(_))));
}

/// A document whose page has a header, its paragraph `33333333`.
fn with_header() -> Arc<[u8]> {
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let body = paragraph("11111111", "body", false)
        + r#"<w:sectPr><w:headerReference w:type="default" r:id="rId1"/></w:sectPr>"#;
    let document = format!("<w:document {NS}><w:body>{body}</w:body></w:document>");
    let header = format!("<w:hdr {NS}>{}</w:hdr>", paragraph("33333333", "head", false));
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/header" Target="header1.xml"/></Relationships>"#
    );
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/></Types>"#;
    let mut w = Writer::new();
    for (name, data) in [
        ("[Content_Types].xml", types.to_owned()),
        ("_rels/.rels", rels.to_owned()),
        ("word/document.xml", document),
        ("word/_rels/document.xml.rels", doc_rels),
        ("word/header1.xml", header),
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

#[test]
fn a_headers_paragraph_is_tagged_with_its_id() {
    let s = session();
    let d = s.open(with_header(), DocKind::Docx, "d.docx").unwrap();
    let lay = layout();
    let view = s.view(&d, None, &*lay, &ctx()).unwrap();
    assert_eq!(view.pages[0].blocks, ["p:11111111"], "the body's blocks only");
    let page = s.page_svg(&d, 1, &view.token, &*lay, TextMode::Text, &ctx()).unwrap();
    assert!(
        page.svg.contains(r#"data-docx-agent-id="header1/p:33333333""#),
        "{}",
        page.svg
    );
    assert!(page.svg.contains(r#"data-docx-agent-id="p:11111111""#));
}

#[test]
fn views_are_read_while_another_thread_edits() {
    let (s, d, lay) = opened(&two_pages());
    let rounds = 6;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for n in 0..rounds {
                let text = format!("edit {n}");
                let edited = call(
                    &s,
                    "word_set_text",
                    json!({"doc": "d1", "target": "p:22222222", "text": text}),
                    &ctx(),
                );
                assert_eq!(edited["ok"], json!(true), "{edited}");
            }
        });
        scope.spawn(|| {
            let mut since: Option<String> = None;
            for _ in 0..rounds {
                let view = s.view(&d, since.as_deref(), &*lay, &ctx()).unwrap();
                assert_eq!(view.pages.len(), 2);
                for &page in &view.changed {
                    let drawn = s
                        .page_svg(&d, page, &view.token, &*lay, TextMode::Text, &ctx())
                        .unwrap();
                    assert!(drawn.svg.starts_with("<svg") || drawn.svg.starts_with("<?xml"));
                }
                assert!(!view.stale);
                since = Some(view.token);
            }
        });
    });
    let last = s.view(&d, None, &*lay, &ctx()).unwrap();
    let page = s.page_svg(&d, 2, &last.token, &*lay, TextMode::Text, &ctx()).unwrap();
    assert!(page.svg.contains(&format!("edit {}", rounds - 1)), "{}", page.svg);
}

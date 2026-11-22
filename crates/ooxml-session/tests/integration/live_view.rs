//! An application's live view of a Word document on the session (lane LV2): every page and
//! its blocks first, then the pages an edit changed and those it added; a page's SVG of the
//! version a token names, tagged with that version's ids (a header's too), drawn after the
//! document moved on; tokens kept for the last four versions; the refusals and errors; views
//! read while another thread edits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{layout, Never, NoFonts, NoParts, Unlimited};
use crate::editing::{call, docx, session};
use ooxml_base::context::Timestamp;
use ooxml_base::{CacheBudget, Clock, Context, Limits, Locale};
use ooxml_session::{CallContext, DocKind, Session, SessionConfig, TextMode, ViewError, VIEW_TOKENS};
use ooxml_tools_word::EnginePages;
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// A clock that always says its seconds past the epoch.
struct At(i64);

impl Clock for At {
    fn now(&self) -> Timestamp {
        Timestamp {
            seconds: self.0,
            nanos: 0,
        }
    }
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

/// A session made at `seconds` past the epoch, its memory budget `budget`.
fn made(seconds: i64, budget: Arc<dyn CacheBudget>) -> Session {
    let context = Context {
        fonts: Arc::new(NoFonts),
        parts: Arc::new(NoParts),
        clock: Arc::new(At(seconds)),
        budget,
        cancel: Arc::new(Never),
        limits: Limits::default(),
        locale: Locale::default(),
    };
    Session::new(SessionConfig::default(), context)
}

/// A session made a second after [`session`]'s.
fn restarted() -> Session {
    made(1_795_000_001, Arc::new(Unlimited))
}

/// A memory budget whose limit a test sets, counting what it granted and how often it was
/// asked.
struct Counted {
    limit: AtomicUsize,
    used: AtomicUsize,
    asked: AtomicUsize,
}

impl Counted {
    fn new() -> Arc<Counted> {
        Arc::new(Counted {
            limit: AtomicUsize::new(usize::MAX),
            used: AtomicUsize::new(0),
            asked: AtomicUsize::new(0),
        })
    }

    fn limit(&self, limit: usize) {
        self.limit.store(limit, Ordering::SeqCst);
    }

    fn within(&self) -> bool {
        self.used() <= self.limit.load(Ordering::SeqCst)
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

impl CacheBudget for Counted {
    fn reserve(&self, bytes: usize) -> bool {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let after = self.used().saturating_add(bytes);
        let fits = after <= self.limit.load(Ordering::SeqCst);
        if fits {
            self.used.store(after, Ordering::SeqCst);
        }
        fits
    }
    fn release(&self, bytes: usize) {
        self.used.fetch_sub(bytes, Ordering::SeqCst);
    }
    fn used(&self) -> usize {
        self.used.load(Ordering::SeqCst)
    }
}

fn set_text(s: &Session, text: &str) {
    let edited = call(
        s,
        "word_set_text",
        json!({"doc": "d1", "target": "p:22222222", "text": text}),
        &ctx(),
    );
    assert_eq!(edited["ok"], json!(true), "{edited}");
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

/// A document of `body` beside `parts`, each `(name in word/, relationship kind, content
/// type's last segment, its XML)`, related from the document as `rId1`, `rId2`...
fn packaged(body: &str, parts: &[(&str, &str, &str, String)]) -> Arc<[u8]> {
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml";
    let document = format!("<w:document {NS}><w:body>{body}</w:body></w:document>");
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#
    );
    let related: String = parts
        .iter()
        .enumerate()
        .map(|(k, (name, kind, _, _))| {
            format!(
                r#"<Relationship Id="rId{}" Type="{REL}/{kind}" Target="{name}"/>"#,
                k + 1
            )
        })
        .collect();
    let doc_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{related}</Relationships>"#
    );
    let overrides: String = parts
        .iter()
        .map(|(name, _, kind, _)| format!(r#"<Override PartName="/word/{name}" ContentType="{TYPE}.{kind}+xml"/>"#))
        .collect();
    let types = format!(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{TYPE}.document.main+xml"/>{overrides}</Types>"#
    );
    let mut files = vec![
        ("[Content_Types].xml".to_owned(), types),
        ("_rels/.rels".to_owned(), rels),
        ("word/document.xml".to_owned(), document),
        ("word/_rels/document.xml.rels".to_owned(), doc_rels),
    ];
    files.extend(
        parts
            .iter()
            .map(|(name, _, _, xml)| (format!("word/{name}"), xml.clone())),
    );
    let mut w = Writer::new();
    for (name, data) in files {
        let entry = NewEntry {
            name,
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

/// A document whose page has a header, its paragraph `33333333`.
fn with_header() -> Arc<[u8]> {
    let body = paragraph("11111111", "body", false)
        + r#"<w:sectPr><w:headerReference w:type="default" r:id="rId1"/></w:sectPr>"#;
    let header = format!("<w:hdr {NS}>{}</w:hdr>", paragraph("33333333", "head", false));
    packaged(&body, &[("header1.xml", "header", "header", header)])
}

/// Styles whose heading (`Heading1`) is `half_points` half-points.
fn styles(half_points: u32) -> String {
    format!(
        r#"<w:styles {NS}><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:rPr><w:sz w:val="{half_points}"/></w:rPr></w:style></w:styles>"#
    )
}

/// A heading `11111111` above two pages.
fn headed() -> String {
    let heading = r#"<w:p w14:paraId="11111110" w14:textId="77777777"><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Title</w:t></w:r></w:p>"#;
    heading.to_owned() + &two_pages()
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

#[test]
fn sessions_made_at_the_same_moment_tell_their_tokens_apart() {
    let one = made(1_795_000_000, Arc::new(Unlimited));
    let two = made(1_795_000_000, Arc::new(Unlimited));
    let a = one.open(docx(&two_pages()), DocKind::Docx, "d.docx").unwrap();
    let b = two.open(docx(&two_pages()), DocKind::Docx, "d.docx").unwrap();
    assert_eq!(a, b, "the same document id in each");
    let lay = layout();
    let theirs = two.view(&b, None, &*lay, &ctx()).unwrap();
    let ours = one.view(&a, None, &*lay, &ctx()).unwrap();
    assert_ne!(ours.token, theirs.token);
    let unknown = one.page_svg(&a, 1, &theirs.token, &*lay, TextMode::Text, &ctx());
    assert!(matches!(unknown, Err(ViewError::UnknownToken { .. })), "{unknown:?}");
}

#[test]
fn a_layout_the_budget_has_no_room_for_lets_older_ones_go_or_is_not_kept() {
    let budget = Counted::new();
    let s = made(1_795_000_000, Arc::clone(&budget) as Arc<dyn CacheBudget>);
    let d = s.open(docx(&two_pages()), DocKind::Docx, "d.docx").unwrap();
    let lay = layout();
    let first = s.view(&d, None, &*lay, &ctx()).unwrap();
    let one = budget.used();
    assert!(one > 0, "the layout kept is reserved");

    // room for about one: the first is let go for the second
    budget.limit(one + one / 2);
    set_text(&s, "deux");
    let second = s.view(&d, Some(&first.token), &*lay, &ctx()).unwrap();
    assert_eq!(second.changed, [2], "compared with the first, held by the call");
    assert!(budget.within() && budget.used() > 0);
    let stale = s.view(&d, Some(&first.token), &*lay, &ctx()).unwrap();
    assert!(stale.stale, "the first is let go");

    // room for none: the third's layout is not kept, the second's let go
    budget.limit(0);
    set_text(&s, "trois");
    let third = s.view(&d, Some(&second.token), &*lay, &ctx()).unwrap();
    assert_eq!(budget.used(), 0);
    let page = s.page_svg(&d, 2, &third.token, &*lay, TextMode::Text, &ctx()).unwrap();
    assert!(page.svg.contains("trois"), "laid out on demand");
    assert_eq!(budget.used(), 0, "nor kept after");
    set_text(&s, "quatre");
    let fourth = s.view(&d, Some(&third.token), &*lay, &ctx()).unwrap();
    assert!(fourth.stale, "a version whose layout was not kept is stale");
    assert_eq!(fourth.changed, [1, 2]);
    assert_eq!(budget.used(), 0);
}

#[test]
fn an_old_versions_pages_are_laid_out_once() {
    let budget = Counted::new();
    let s = made(1_795_000_000, Arc::clone(&budget) as Arc<dyn CacheBudget>);
    let d = s.open(docx(&two_pages()), DocKind::Docx, "d.docx").unwrap();
    let view = s.view(&d, None, &*layout(), &ctx()).unwrap();
    let kept = budget.used();
    // another engine (a host's after a restart of its engine) does not paint the version
    let other = layout();
    let asked = budget.asked();
    let first = s.page_svg(&d, 1, &view.token, &*other, TextMode::Text, &ctx()).unwrap();
    assert!(first.svg.contains(r#"data-docx-agent-id="p:11111111""#));
    assert_eq!(budget.asked(), asked + 1, "the spare layout is reserved");
    let spare = budget.used() - kept;
    assert!(spare > 0);
    let second = s.page_svg(&d, 2, &view.token, &*other, TextMode::Text, &ctx()).unwrap();
    assert!(second.svg.contains("two"));
    assert_eq!(
        budget.asked(),
        asked + 1,
        "the next page is painted from it, not laid out again"
    );
    assert_eq!(budget.used(), kept + spare);
    // let go with its version
    for n in 0..VIEW_TOKENS {
        set_text(&s, &format!("two {n}"));
        s.view(&d, None, &*layout(), &ctx()).unwrap();
    }
    assert!(s.view(&d, Some(&view.token), &*layout(), &ctx()).unwrap().stale);
    drop(s);
    assert_eq!(budget.used(), 0, "every reservation given back");
}

#[test]
fn an_older_version_is_drawn_with_its_own_styles() {
    let original = || packaged(&headed(), &[("styles.xml", "styles", "styles", styles(32))]);
    let lay = layout();
    // the first version's page as drawn by a session that never saw the edit
    let fresh = session();
    let f = fresh.open(original(), DocKind::Docx, "d.docx").unwrap();
    let token = fresh.view(&f, None, &*lay, &ctx()).unwrap().token;
    let before = fresh.page_svg(&f, 1, &token, &*lay, TextMode::Text, &ctx()).unwrap();

    let s = session();
    let d = s.open(original(), DocKind::Docx, "d.docx").unwrap();
    let lay = layout();
    let first = s.view(&d, None, &*lay, &ctx()).unwrap();
    let larger = vec![("word/styles.xml".to_owned(), Some(Arc::from(styles(48).as_bytes())))];
    s.update(&d, larger, &ctx()).unwrap();
    let second = s.view(&d, Some(&first.token), &*lay, &ctx()).unwrap();
    assert!(second.changed.contains(&1), "the heading's page");
    let now = s.page_svg(&d, 1, &second.token, &*lay, TextMode::Text, &ctx()).unwrap();
    assert_ne!(now.svg, before.svg);
    // first drawn after the styles changed: not with the engine's newer styles
    let old = s.page_svg(&d, 1, &first.token, &*lay, TextMode::Text, &ctx()).unwrap();
    assert_eq!(old.svg, before.svg, "the first version, with its own styles");
}

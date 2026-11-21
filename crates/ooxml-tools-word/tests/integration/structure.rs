//! The structure tools' definitions: the golden exports docx-agent's Python test holds to
//! today's (`test_rust_structure.py`), and their argument rules (a place named another
//! placing tool's way read as the tool's own, or refused with the call that works).
//! Their handlers' equality with docx-agent is `crates/ooxml-oracle/python/word_edits_diff.py
//! --structure`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited, R, W};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::{ToolArgs, ToolResult};
use ooxml_tools_word::{
    word_copy_from, word_fields, word_links, word_notes, CallEditor, Editing, WordHeadersFooters, WordLinks, WordMove,
    WordNotes, WordSections, KIND,
};
use ooxml_word_edit::{Editor, FieldLayout, FieldPlace, FieldRun, LaidFields, PageShows, Unknown};
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::Arc;

/// A made-up document: two paragraphs, a bookmark over the first one's word "Alpha".
fn package() -> Package {
    let body = concat!(
        r#"<w:p w14:paraId="10000001" w14:textId="20000001"><w:bookmarkStart w:id="0" w:name="First"/>"#,
        r#"<w:r><w:t>Alpha</w:t></w:r><w:bookmarkEnd w:id="0"/><w:r><w:t xml:space="preserve"> beta gamma</w:t></w:r></w:p>"#,
        r#"<w:p w14:paraId="10000002" w14:textId="20000002"><w:r><w:t>Delta epsilon</w:t></w:r></w:p>"#,
    );
    package_of(body, "")
}

/// A made-up document of `body`, its styles Normal and `styles`.
fn package_of(body: &str, styles: &str) -> Package {
    package_numbered(body, styles, "")
}

/// [`package_of`] with a numbering part holding `numbering` (none where empty).
fn package_numbered(body: &str, styles: &str, numbering: &str) -> Package {
    let numbering_rel = if numbering.is_empty() {
        String::new()
    } else {
        format!(r#"<Relationship Id="rId2" Type="{R}numbering" Target="numbering.xml"/>"#)
    };
    let mut parts = vec![
        ("[Content_Types].xml".to_owned(), r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#.to_owned()),
        ("word/_rels/document.xml.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}styles" Target="styles.xml"/>{numbering_rel}</Relationships>"#)),
        ("word/styles.xml".to_owned(), format!(r#"<w:styles {W}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>{styles}</w:styles>"#)),
        ("_rels/.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/></Relationships>"#)),
        ("word/document.xml".to_owned(), format!("<w:document {W}><w:body>{body}</w:body></w:document>")),
    ];
    if !numbering.is_empty() {
        parts.push((
            "word/numbering.xml".to_owned(),
            format!("<w:numbering {W}>{numbering}</w:numbering>"),
        ));
    }
    let mut w = Writer::new();
    for (name, data) in parts {
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
    Package::open(Arc::from(w.finish(b"").unwrap()), &Limits::default()).unwrap()
}

/// A layout of two pages, the first paragraph on page 1, the second on page 2, numbered
/// from 1 in one section.
struct TwoPages;

struct LaidTwo;

impl LaidFields for LaidTwo {
    fn page_field(&self, instruction: &str, keyword: &str, at: FieldPlace<'_>) -> Result<String, Unknown> {
        let page = match at {
            FieldPlace::Body("p:10000001") | FieldPlace::Page(0) => 1,
            FieldPlace::Body("p:10000002") | FieldPlace::Page(1) => 2,
            _ => return Err(Unknown("not drawn".into())),
        };
        let _ = instruction;
        Ok(match keyword {
            "PAGE" => page.to_string(),
            _ => "2".to_owned(),
        })
    }

    fn page_reference(&self, instruction: &str, at: FieldPlace<'_>) -> Result<String, Unknown> {
        self.page_field(instruction, "PAGE", at)
    }

    fn style_reference(&self, _: &str, _: FieldPlace<'_>, _: FieldRun<'_>) -> Result<String, Unknown> {
        Err(Unknown("not drawn".into()))
    }

    fn pages(&self) -> Vec<PageShows> {
        vec![
            PageShows {
                section: 0,
                kind: "default",
                blank: false,
            },
            PageShows {
                section: 0,
                kind: "default",
                blank: false,
            },
        ]
    }
}

impl FieldLayout for TwoPages {
    fn lay_out(&self, _: &Package) -> Result<Box<dyn LaidFields + '_>, String> {
        Ok(Box::new(LaidTwo))
    }
}

/// `tool` called with `arguments` on a fresh copy of the made-up document.
fn call<A: ToolArgs>(arguments: Value, tool: impl Fn(&A, &Editing<'_>) -> ToolResult<Value>) -> (Value, Editor) {
    call_tracking(arguments, None, tool)
}

/// [`call`] with the host's tracking mode.
fn call_tracking<A: ToolArgs>(
    arguments: Value,
    tracking: Option<&ooxml_word_edit::Tracking>,
    tool: impl Fn(&A, &Editing<'_>) -> ToolResult<Value>,
) -> (Value, Editor) {
    call_on(package(), arguments, tracking, tool)
}

/// [`call_tracking`] on `package`.
fn call_on<A: ToolArgs>(
    package: Package,
    arguments: Value,
    tracking: Option<&ooxml_word_edit::Tracking>,
    tool: impl Fn(&A, &Editing<'_>) -> ToolResult<Value>,
) -> (Value, Editor) {
    let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let reply = {
        let call_editor = CallEditor::new(&editor);
        let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z")
            .with_tracking(tracking)
            .with_layout(Some(&TwoPages));
        let (args, _) = A::accept(arguments, Some("docx")).unwrap();
        Value::Object(tool(&args, &editing).to_json())
    };
    (reply, editor)
}

#[test]
fn the_unregistered_definitions_are_the_golden_exports() {
    // a tool the session registers is held to its golden export there (ooxml-session's
    // tests/integration/registry.rs)
    for (spec, golden) in [
        (WordMove::spec(), include_str!("../golden/word_move.json")),
        (WordSections::spec(), include_str!("../golden/word_sections.json")),
        (
            WordHeadersFooters::spec(),
            include_str!("../golden/word_headers_footers.json"),
        ),
        (WordNotes::spec(), include_str!("../golden/word_notes.json")),
        (WordLinks::spec(), include_str!("../golden/word_links.json")),
    ] {
        assert!(spec.problems().is_empty(), "{}: {:?}", spec.name, spec.problems());
        assert_eq!(format!("{}\n", spec.export(Some(&[KIND]))), golden, "{}", spec.name);
    }
}

#[test]
fn a_place_named_another_tools_way_is_read_as_its_own() {
    let (call, notes) = WordMove::accept(json!({"doc": "d1", "blocks": "p:A", "at": "before:p:B"}), None).unwrap();
    assert_eq!(call.after.as_deref(), Some("before:p:B"));
    assert_eq!(notes, ["at read as after"]);
    let (call, notes) = WordNotes::accept(json!({"doc": "d1", "action": "insert", "after": "p:A@3"}), None).unwrap();
    assert_eq!(call.at.as_deref(), Some("p:A@3"));
    assert_eq!(notes, ["after read as at"]);
}

#[test]
fn a_place_whose_side_is_in_its_name_is_refused_with_the_call() {
    let error = WordNotes::accept(json!({"doc": "d1", "action": "insert", "before": "p:A@3:9"}), None).unwrap_err();
    assert_eq!(error.field(), Some("before"));
    let call = error.way_out().and_then(|w| w.try_call()).unwrap();
    assert_eq!(call.arguments["at"], json!("p:A@3"));
    assert!(call.arguments.get("before").is_none());
}

#[test]
fn a_bookmark_marking_its_text_already_is_no_change() {
    let (reply, editor) = call(
        json!({"doc": "d1", "action": "add_bookmark", "target": "p:10000001@0:5", "bookmark": "first"}),
        word_links,
    );
    assert_eq!(reply["summary"], json!("No change: bm:First marks this text already"));
    assert_eq!(editor.store().snapshot().doc().package().part_names().len(), 5);
}

#[test]
fn a_bookmark_and_a_hyperlink_are_made_and_listed() {
    let (reply, _) = call(
        json!({"doc": "d1", "action": "add_bookmark", "find": "epsilon", "bookmark": "Second"}),
        word_links,
    );
    assert_eq!(reply["created"], json!(["bm:Second"]));
    let (reply, _) = call(
        json!({"doc": "d1", "action": "add_hyperlink", "target": "p:10000002@0:5", "bookmark": "First"}),
        word_links,
    );
    assert_eq!(reply["created"], json!(["hl:p:10000002/0"]));
}

/// docx-agent's `_add_hyperlink_tracked`: the range's runs deleted, a new link after them
/// holding their copies in an insertion; the paragraph's textId drawn again.
#[test]
fn a_hyperlink_made_while_tracking_deletes_and_inserts_the_range() {
    let tracking = ooxml_word_edit::Tracking {
        author: "Writer A".to_owned(),
        date: "2026-11-17T10:00:00Z".to_owned(),
    };
    let (reply, editor) = call_tracking(
        json!({"doc": "d1", "action": "add_hyperlink", "target": "p:10000002@0:5", "url": "https://example.com/"}),
        Some(&tracking),
        word_links,
    );
    assert_eq!(reply["created"], json!(["hl:p:10000002/0"]), "{reply}");
    let tree = editor
        .store()
        .snapshot()
        .doc()
        .tree("word/document.xml")
        .unwrap()
        .unwrap();
    let xml = String::from_utf8(tree.write()).unwrap();
    let paragraph = &xml[xml.find(r#"w14:paraId="10000002""#).unwrap()..];
    let deleted = paragraph.find("<w:del ").unwrap();
    let link = paragraph.find("<w:hyperlink ").unwrap();
    assert!(deleted < link, "the range deleted, then the link: {paragraph}");
    assert!(paragraph[deleted..link].contains("<w:delText>Delta</w:delText>"));
    let inside = &paragraph[link..paragraph.find("</w:hyperlink>").unwrap()];
    assert!(
        inside.contains(r#"<w:ins w:id="#) && inside.contains(r#"w:author="Writer A""#),
        "{inside}"
    );
    assert!(inside.contains(r#"<w:rStyle w:val="Hyperlink"/>"#) && inside.contains("<w:t>Delta</w:t>"));
    assert!(
        !paragraph.starts_with(r#"w14:paraId="10000002" w14:textId="20000002""#),
        "textId drawn again"
    );
}

#[test]
fn refusals_carry_their_way_out() {
    let (reply, _) = call(
        json!({"doc": "d1", "action": "add_bookmark", "target": "p:10000001", "bookmark": "1st"}),
        word_links,
    );
    assert_eq!(reply["error"]["code"], json!("refused"));
    let (reply, _) = call(
        json!({"doc": "d1", "action": "add_bookmark", "bookmark": "B"}),
        word_links,
    );
    assert_eq!(reply["error"]["code"], json!("invalid_arguments"));
    assert!(reply["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("give exactly one of target, find"));
    let (reply, _) = call(
        json!({"doc": "d1", "action": "insert", "find": "a", "text": "x"}),
        word_notes,
    );
    assert_eq!(reply["error"]["code"], json!("ambiguous"));
}

#[test]
fn a_first_footnote_brings_the_notes_parts() {
    let (reply, editor) = call(
        json!({"doc": "d1", "action": "insert", "at": "p:10000001@5", "text": "A source."}),
        word_notes,
    );
    assert_eq!(reply["summary"], json!("Inserted fn:1"));
    let head = editor.store().snapshot();
    let names = head.doc().package().part_names();
    assert!(names.iter().any(|n| n == "word/footnotes.xml"));
    assert!(names.iter().any(|n| n == "word/endnotes.xml"));
    let (reply, _) = call(json!({"doc": "d1", "action": "list"}), word_notes);
    assert_eq!(reply["data"], json!([]));
}

#[test]
fn a_page_number_is_inserted_with_its_result() {
    let (reply, editor) = call(
        json!({"doc": "d1", "action": "insert_field", "at": "p:10000002@5", "instruction": "PAGE"}),
        word_fields,
    );
    assert_eq!(reply["summary"], json!("Inserted field fld:body/0"));
    assert_eq!(reply["created"], json!(["fld:body/0"]));
    let head = editor.store().snapshot();
    let found = ooxml_word_edit::fields(head.doc()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].keyword.as_str(), found[0].result.as_str()), ("PAGE", "2"));
}

/// docx-agent writes a `HYPERLINK` field as Word saves one: its text inserted and made a
/// `w:hyperlink` (`insert_hyperlink`); the reply names the link.
#[test]
fn a_hyperlink_field_is_written_as_a_hyperlink() {
    let (reply, editor) = call(
        json!({"doc": "d1", "action": "insert_field", "at": "p:10000002@0",
               "instruction": "HYPERLINK \"https://example.com/\""}),
        word_fields,
    );
    assert_eq!(reply["created"], json!(["hl:p:10000002/0"]), "{reply}");
    let tree = editor
        .store()
        .snapshot()
        .doc()
        .tree("word/document.xml")
        .unwrap()
        .unwrap();
    let xml = String::from_utf8(tree.write()).unwrap();
    let link = &xml[xml.find("<w:hyperlink ").unwrap()..xml.find("</w:hyperlink>").unwrap()];
    assert!(link.contains("<w:t>https://example.com/</w:t>"), "{link}");
    assert!(!xml.contains("HYPERLINK"), "no field is written");
}

#[test]
fn a_date_shows_the_calls_day_and_a_sequence_counts() {
    let (reply, editor) = call(
        json!({"doc": "d1", "action": "insert_date", "at": "p:10000001@0"}),
        word_fields,
    );
    assert_eq!(reply["created"], json!(["fld:body/0"]));
    let head = editor.store().snapshot();
    let found = ooxml_word_edit::fields(head.doc()).unwrap();
    assert_eq!(found[0].result, "17 November 2026");
    let (reply, _) = call(
        json!({"doc": "d1", "action": "insert_caption", "after": "p:10000002", "label": "Table", "text": ": Results"}),
        word_fields,
    );
    assert_eq!(reply["summary"], json!("Inserted a caption"));
}

#[test]
fn a_table_of_contents_with_no_headings_says_so() {
    let (reply, editor) = call(
        json!({"doc": "d1", "action": "insert_toc", "after": "p:10000002"}),
        word_fields,
    );
    assert_eq!(reply["summary"], json!("Inserted a table of contents fld:body/0"));
    let warnings = reply["warnings"].as_array().unwrap();
    assert!(warnings[0]
        .as_str()
        .unwrap()
        .contains("the table of contents has no entries"));
    let head = editor.store().snapshot();
    let found = ooxml_word_edit::fields(head.doc()).unwrap();
    assert_eq!(found[0].result, "No table of contents entries found.");
}

#[test]
fn a_table_of_contents_numbers_its_new_bookmarks_on_from_the_largest() {
    // Three headings: the second has a `_Toc` bookmark of its own (kept), the others get new
    // ones numbered one above the largest in the document, in order.
    let heading = |id: u32, inside: &str| {
        format!(
            r#"<w:p w14:paraId="{id:08X}" w14:textId="20000001"><w:pPr><w:pStyle w:val="Heading1"/></w:pPr>{inside}</w:p>"#
        )
    };
    let body = [
        heading(0x1000_0001, "<w:r><w:t>One</w:t></w:r>"),
        heading(
            0x1000_0002,
            r#"<w:bookmarkStart w:id="5" w:name="_Toc100000041"/><w:r><w:t>Two</w:t></w:r><w:bookmarkEnd w:id="5"/>"#,
        ),
        heading(0x1000_0003, "<w:r><w:t>Three</w:t></w:r>"),
        r#"<w:p w14:paraId="10000004" w14:textId="20000001"><w:bookmarkStart w:id="6" w:name="_Toc100000007"/><w:r><w:t>Body</w:t></w:r><w:bookmarkEnd w:id="6"/></w:p>"#.to_owned(),
    ]
    .concat();
    let styles = concat!(
        r#"<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/>"#,
        r#"<w:pPr><w:outlineLvl w:val="0"/></w:pPr></w:style>"#,
    );
    let (reply, editor) = call_on(
        package_of(&body, styles),
        json!({"doc": "d1", "action": "insert_toc", "after": "p:10000004"}),
        None,
        word_fields,
    );
    assert_eq!(
        reply["summary"],
        json!("Inserted a table of contents fld:body/0"),
        "{reply}"
    );
    let head = editor.store().snapshot();
    let xml = String::from_utf8(
        head.doc()
            .package()
            .read("word/document.xml")
            .unwrap()
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let names: Vec<&str> = xml
        .split(r#"w:name="_Toc"#)
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap())
        .collect();
    assert_eq!(names, ["100000042", "100000041", "100000043", "100000007"], "{xml}");
    // Their annotation ids one above the document's largest (6), one after another.
    let ids: Vec<&str> = xml
        .split("<w:bookmarkStart ")
        .skip(1)
        .map(|rest| rest.split(r#"w:id=""#).nth(1).unwrap().split('"').next().unwrap())
        .collect();
    assert_eq!(ids, ["7", "5", "8", "6"], "{xml}");
}

#[test]
fn a_numbered_heading_s_entry_shows_its_number() {
    // Level 1 is followed by a tab (Word's default): the number in its own run, a tab run,
    // the text, with a left stop after the number; level 2 by a space: one run.
    let heading = |id: u32, level: u32, text: &str| {
        format!(
            r#"<w:p w14:paraId="{id:08X}" w14:textId="20000001"><w:pPr><w:pStyle w:val="Heading{level}"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        )
    };
    let body = [
        heading(0x1000_0001, 1, "One"),
        heading(0x1000_0002, 2, "Two"),
        r#"<w:p w14:paraId="10000004" w14:textId="20000001"><w:r><w:t>Body</w:t></w:r></w:p>"#.to_owned(),
    ]
    .concat();
    let style = |level: u32| {
        format!(
            r#"<w:style w:type="paragraph" w:styleId="Heading{level}"><w:name w:val="heading {level}"/><w:basedOn w:val="Normal"/><w:pPr><w:numPr><w:ilvl w:val="{}"/><w:numId w:val="1"/></w:numPr><w:outlineLvl w:val="{}"/></w:pPr></w:style>"#,
            level - 1,
            level - 1
        )
    };
    let numbering = concat!(
        r#"<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="multilevel"/>"#,
        r#"<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:pStyle w:val="Heading1"/><w:lvlText w:val="%1."/></w:lvl>"#,
        r#"<w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:pStyle w:val="Heading2"/><w:suff w:val="space"/><w:lvlText w:val="%1.%2"/></w:lvl>"#,
        r#"</w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#,
    );
    let (reply, editor) = call_on(
        package_numbered(&body, &[style(1), style(2)].concat(), numbering),
        json!({"doc": "d1", "action": "insert_toc", "after": "p:10000004"}),
        None,
        word_fields,
    );
    assert_eq!(
        reply["summary"],
        json!("Inserted a table of contents fld:body/0"),
        "{reply}"
    );
    let head = editor.store().snapshot();
    let xml = String::from_utf8(
        head.doc()
            .package()
            .read("word/document.xml")
            .unwrap()
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(xml.contains(r#"<w:tab w:val="left" w:pos=""#), "{xml}");
    assert!(
        xml.contains("<w:t>1.</w:t></w:r><w:r><w:rPr><w:noProof/></w:rPr><w:tab/></w:r>"),
        "{xml}"
    );
    assert!(xml.contains("<w:t>1.1 Two</w:t>"), "{xml}");
}

#[test]
fn field_refusals_carry_their_way_out() {
    let (reply, _) = call(
        json!({"doc": "d1", "action": "update", "ids": ["fld:body/9"]}),
        word_fields,
    );
    assert_eq!(reply["error"]["code"], json!("refused"));
    assert!(
        reply["error"]["way_out"].is_object()
            || reply["error"]["try_call"].is_object()
            || reply["error"].get("valid_options").is_some()
            || reply["error"].as_object().unwrap().len() > 2
    );
    let (reply, _) = call(
        json!({"doc": "d1", "action": "insert_toc", "after": "p:10000002", "levels": "3-1"}),
        word_fields,
    );
    assert_eq!(reply["error"]["code"], json!("invalid_arguments"));
    assert_eq!(reply["error"]["field"], json!("levels"));
    let (reply, _) = call(
        json!({"doc": "d1", "action": "insert_field", "at": "p:10000001@99", "instruction": "PAGE"}),
        word_fields,
    );
    assert_eq!(reply["error"]["code"], json!("refused"));
}

#[test]
fn an_update_with_nothing_to_change_is_no_change() {
    let (reply, _) = call(json!({"doc": "d1", "action": "update"}), word_fields);
    assert_eq!(reply["summary"], json!("Updated fields: 0 result(s) changed"));
    assert!(reply.get("changed").is_none_or(|c| c == &json!([])));
}

#[test]
fn blocks_of_the_document_are_copied_with_fresh_ids() {
    let (reply, editor) = call(
        json!({"doc": "d1", "source_doc": "d1", "range": "p:10000002", "at": "end", "ref": "copy"}),
        word_copy_from,
    );
    assert_eq!(reply["summary"], json!("Copied 1 block(s) from d1"), "{reply}");
    let made = reply["data"]["blocks"][0].as_str().unwrap().to_owned();
    assert_ne!(made, "p:10000002");
    assert_eq!(reply["data"]["copied"], json!({"p:10000002": made}));
    assert_eq!(reply["created"], json!([made]));
    assert_eq!(reply["refs"], json!({"copy": made}));
    let head = editor.store().snapshot();
    let body = head.doc().package().read("word/document.xml").unwrap().unwrap();
    let text = String::from_utf8_lossy(&body);
    assert_eq!(text.matches("Delta epsilon").count(), 2);
}

#[test]
fn what_the_copy_does_not_port_is_left_to_the_host() {
    // a place given by reference: the host resolves it before it calls
    let (reply, editor) = call(
        json!({"doc": "d1", "source_doc": "d1", "range": "p:10000002", "at": "after:$mark"}),
        word_copy_from,
    );
    // `unported` is the host's to read: the reply changes nothing and says nothing
    assert_eq!(reply["summary"], json!(""), "{reply}");
    assert_eq!(reply["changed"], json!([]), "{reply}");
    let body = editor
        .store()
        .snapshot()
        .doc()
        .package()
        .read("word/document.xml")
        .unwrap()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&body).matches("Delta epsilon").count(), 1);
}

#[test]
fn a_bookmarked_block_and_a_tracked_copy_are_copied_in_rust() {
    for (arguments, tracked) in [
        (
            json!({"doc": "d1", "source_doc": "d1", "range": "p:10000001", "at": "end"}),
            false,
        ),
        (
            json!({"doc": "d1", "source_doc": "d1", "range": "p:10000002", "at": "end"}),
            true,
        ),
    ] {
        let tracking = tracked.then(|| ooxml_word_edit::Tracking {
            author: "A".to_owned(),
            date: "2026-11-17T10:00:00Z".to_owned(),
        });
        let (reply, editor) = call_tracking(arguments, tracking.as_ref(), word_copy_from);
        assert_eq!(reply["summary"], json!("Copied 1 block(s) from d1"), "{reply}");
        assert_eq!(reply["created"].as_array().map(Vec::len), Some(1), "{reply}");
        let body = editor
            .store()
            .snapshot()
            .doc()
            .package()
            .read("word/document.xml")
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert_eq!(text.contains("<w:ins "), tracked, "{text}");
    }
}

#[test]
fn a_copy_takes_the_other_style_policies() {
    for styles in ["keep_source", "merge"] {
        let (reply, editor) = call(
            json!({"doc": "d1", "source_doc": "d1", "range": "p:10000002", "at": "end", "styles": styles}),
            word_copy_from,
        );
        assert!(reply.get("error").is_none(), "{reply}");
        let body = editor
            .store()
            .snapshot()
            .doc()
            .package()
            .read("word/document.xml")
            .unwrap()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&body).matches("Delta epsilon").count(),
            2,
            "{styles}"
        );
    }
}

#[test]
fn a_copy_refuses_a_ref_naming_a_place_and_an_unknown_block() {
    let (reply, _) = call(
        json!({"doc": "d1", "source_doc": "d1", "range": "p:10000002", "at": "end", "ref": "after:x"}),
        word_copy_from,
    );
    assert_eq!(reply["error"]["code"], json!("invalid_arguments"), "{reply}");
    assert_eq!(reply["error"]["field"], json!("ref"));
    let (reply, _) = call(
        json!({"doc": "d1", "source_doc": "d1", "range": "p:00000009", "at": "end"}),
        word_copy_from,
    );
    assert_eq!(reply["error"]["code"], json!("not_found"), "{reply}");
}

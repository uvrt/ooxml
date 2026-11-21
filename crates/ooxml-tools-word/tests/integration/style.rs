//! The style tools' definitions: the golden exports docx-agent's Python test holds to
//! today's (`test_rust_style.py`), and the rules their replies keep (a no-op said, a
//! refusal with its way out). Their handlers' equality with docx-agent is
//! `crates/ooxml-oracle/python/word_edits_diff.py --style`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited, R, W};
use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::{ToolArgs, ToolResult};
use ooxml_tools_word::{
    word_format, word_lists, word_styles, word_template, CallEditor, Editing, WordCompareBlocks, WordTemplate,
};
use ooxml_word_edit::{Editor, FieldLayout, LaidFields, LayOutError, PageReflow};
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::Arc;

/// A made-up document: three paragraphs, the last in a style that numbers it.
fn package() -> Package {
    let body = concat!(
        r#"<w:p w14:paraId="10000001" w14:textId="20000001"><w:r><w:t>Alpha</w:t></w:r></w:p>"#,
        r#"<w:p w14:paraId="10000002" w14:textId="20000002"><w:r><w:t>Beta</w:t></w:r></w:p>"#,
        r#"<w:p w14:paraId="10000003" w14:textId="20000003"><w:pPr><w:pStyle w:val="Numbered"/></w:pPr><w:r><w:t>Gamma</w:t></w:r></w:p>"#,
    );
    let parts = [
        ("[Content_Types].xml".to_owned(), r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#.to_owned()),
        ("word/_rels/document.xml.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}styles" Target="styles.xml"/><Relationship Id="rId2" Type="{R}numbering" Target="numbering.xml"/></Relationships>"#)),
        ("word/styles.xml".to_owned(), format!(r#"<w:styles {W}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Numbered"><w:name w:val="Numbered"/><w:basedOn w:val="Normal"/><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:style></w:styles>"#)),
        ("word/numbering.xml".to_owned(), format!(r#"<w:numbering {W}><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#)),
        ("_rels/.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/></Relationships>"#)),
        ("word/document.xml".to_owned(), format!("<w:document {W}><w:body>{body}</w:body></w:document>")),
    ];
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

/// `tool` called with `arguments` on a fresh copy of the made-up document.
fn call<A: ToolArgs>(arguments: Value, tool: impl Fn(&A, &Editing<'_>) -> ToolResult<Value>) -> (Value, Editor) {
    let editor = Editor::open(package(), StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let reply = {
        let call_editor = CallEditor::new(&editor);
        let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z");
        let (args, _) = A::accept(arguments, Some("docx")).unwrap();
        Value::Object(tool(&args, &editing).to_json())
    };
    (reply, editor)
}

#[test]
fn a_reference_in_another_document_is_one_of_the_calls_documents() {
    let spec = WordCompareBlocks::spec();
    let call = |reference: Value| {
        let arguments = json!({"doc": "d1", "blocks": ["t:1"], "reference": reference});
        ooxml_tools::documents::named(&spec.documents, arguments.as_object().unwrap())
    };
    assert_eq!(call(json!({"block": "t:2"})), ["d1"]);
    assert_eq!(call(json!({"doc": "", "block": "t:2"})), ["d1"]);
    assert_eq!(call(json!({"doc": "d2", "block": "t:2"})), ["d1", "d2"]);
    assert!(spec.problems().is_empty(), "{:?}", spec.problems());
}

#[test]
fn paragraphs_join_a_new_list_and_are_listed() {
    let (reply, _) = call(
        json!({"doc": "d1", "targets": ["p:10000001", "p:10000002"], "action": "add"}),
        word_lists,
    );
    assert_eq!(reply["summary"], json!("add: 2 paragraph(s)"));
    assert_eq!(
        reply["data"][1]["list"],
        json!({"kind": "bullet", "level": 0, "num_id": 2})
    );
    assert_eq!(reply["data"][0]["list"]["num_id"], reply["data"][1]["list"]["num_id"]);
}

#[test]
fn a_paragraph_in_no_list_is_left_as_it_was_and_said() {
    let (reply, _) = call(
        json!({"doc": "d1", "targets": ["p:10000001"], "action": "remove"}),
        word_lists,
    );
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(
        reply["warnings"],
        json!(["1 paragraph(s) were not in a list, left as they were: p:10000001"])
    );
}

#[test]
fn a_style_numbered_paragraph_is_refused_with_the_styles_and_force() {
    let (reply, _) = call(
        json!({"doc": "d1", "targets": ["p:10000003"], "action": "remove"}),
        word_lists,
    );
    assert_eq!(reply["error"]["code"], json!("invalid_arguments"));
    assert_eq!(reply["error"]["field"], json!("targets"));
    assert_eq!(reply["error"]["valid_options"], json!(["Normal", "force"]));
    let (forced, _) = call(
        json!({"doc": "d1", "targets": ["p:10000003"], "action": "remove", "force": true}),
        word_lists,
    );
    assert_eq!(forced["data"][0]["list"], Value::Null);
}

/// The main part's XML after `editor`'s last edit.
fn body(editor: &Editor) -> String {
    let head = editor.store().snapshot();
    let tree = head.doc().tree("word/document.xml").unwrap().unwrap();
    String::from_utf8(tree.write()).unwrap()
}

#[test]
fn a_paragraph_is_formatted_with_its_runs_and_mark() {
    let (reply, editor) = call(
        json!({"doc": "d1", "targets": "p:10000001", "bold": true, "size": 13, "space_before": 6}),
        word_format,
    );
    assert_eq!(reply["summary"], json!("Formatted 1 target(s)"), "{reply}");
    assert_eq!(reply["changed"], json!(["p:10000001"]));
    let xml = body(&editor);
    assert!(xml.contains(r#"<w:pPr><w:spacing w:before="120"/><w:rPr><w:b/><w:bCs/><w:sz w:val="26"/><w:szCs w:val="26"/></w:rPr></w:pPr><w:r><w:rPr><w:b/><w:bCs/><w:sz w:val="26"/><w:szCs w:val="26"/></w:rPr><w:t>Alpha</w:t>"#), "{xml}");
}

#[test]
fn nothing_to_set_is_refused_with_what_can_be_set() {
    let (reply, _) = call(json!({"doc": "d1", "targets": ["p:10000001"]}), word_format);
    assert_eq!(reply["error"]["code"], json!("invalid_arguments"));
    let options = reply["error"]["valid_options"].as_array().unwrap();
    assert_eq!(options.first(), Some(&json!("like")));
    assert_eq!(options.len(), 24);
    let (reply, _) = call(
        json!({"doc": "d1", "targets": ["p:10000001"], "bold": true, "clear": ["bold"]}),
        word_format,
    );
    assert_eq!(reply["error"]["field"], json!("clear"));
    assert!(reply["error"]["try_call"].is_object(), "{reply}");
}

#[test]
fn a_paragraph_already_like_the_sample_is_said_not_changed() {
    let (reply, _) = call(
        json!({"doc": "d1", "targets": ["p:10000002"], "like": "p:10000001"}),
        word_format,
    );
    assert_eq!(reply["changed"], json!([]), "{reply}");
    assert_eq!(reply["data"]["like"]["alike"], json!(["p:10000002"]));
    assert_eq!(
        reply["warnings"],
        json!(["every target already looks like p:10000001: nothing changed"])
    );
    let (reply, editor) = call(
        json!({"doc": "d1", "targets": ["p:10000001"], "like": "p:10000003"}),
        word_format,
    );
    assert_eq!(
        reply["data"]["like"]["set"],
        json!({"p:10000001": ["style"]}),
        "{reply}"
    );
    assert!(body(&editor)
        .contains(r#"<w:p w14:paraId="10000001" w14:textId="20000001"><w:pPr><w:pStyle w:val="Numbered"/></w:pPr>"#));
}

#[test]
fn text_found_more_than_once_is_refused_with_each_place() {
    let (reply, _) = call(json!({"doc": "d1", "find": {"text": "a"}, "italic": true}), word_format);
    assert_eq!(reply["error"]["code"], json!("ambiguous"), "{reply}");
    assert_eq!(
        reply["error"]["valid_options"][0],
        json!("occurrence 1: p:10000001@4:5 'Alph[a]'")
    );
    let (reply, _) = call(
        json!({"doc": "d1", "find": {"text": "a", "expect": "all"}, "italic": true}),
        word_format,
    );
    assert_eq!(reply["data"]["ranges"].as_array().map(Vec::len), Some(4), "{reply}");
}

fn styles(arguments: Value) -> (Value, Editor) {
    call(arguments, word_styles)
}

fn sheet(editor: &Editor) -> String {
    let head = editor.store().snapshot();
    let tree = head.doc().tree("word/styles.xml").unwrap().unwrap();
    String::from_utf8(tree.write()).unwrap()
}

#[test]
fn a_paragraph_style_is_added_with_its_formatting_and_described() {
    let (reply, editor) = styles(
        json!({"doc": "d1", "action": "add", "name": "Lane Body", "based_on": "Normal",
                                         "bold": true, "indent_first": -12, "tabs": [{"position": 72, "alignment": "right"}]}),
    );
    assert_eq!(reply["created"], json!(["style:LaneBody"]), "{reply}");
    let xml = sheet(&editor);
    assert!(xml.contains(r#"<w:style w:type="paragraph" w:customStyle="1" w:styleId="LaneBody"><w:name w:val="Lane Body"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:tabs><w:tab w:val="right" w:pos="1440"/></w:tabs><w:ind w:hanging="240"/></w:pPr><w:rPr><w:b/><w:bCs/></w:rPr></w:style>"#), "{xml}");
}

#[test]
fn regions_without_a_kind_add_a_table_style_and_say_so() {
    // rule 1: regions say a table style
    let (reply, _) = styles(json!({"doc": "d1", "action": "add", "name": "Grid",
                                    "regions": [{"region": "header_row", "bold": true}]}));
    assert_eq!(
        reply["warnings"][0],
        json!("regions given: added as a table style (kind table)"),
        "{reply}"
    );
    assert_eq!(reply["data"]["regions"]["header_row"], json!({"bold": true}));
}

#[test]
fn a_used_style_is_refused_without_a_replacement_and_an_unknown_one_lists_the_styles() {
    // rule 2: the refusal carries the way out
    let (reply, _) = styles(json!({"doc": "d1", "action": "remove", "name": "Numbered"}));
    assert_eq!(reply["error"]["code"], json!("refused"), "{reply}");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("give replacement="),
        "{reply}"
    );
    let (reply, _) = styles(json!({"doc": "d1", "action": "describe", "name": "Nothing"}));
    assert_eq!(reply["error"]["code"], json!("not_found"));
    assert_eq!(reply["error"]["valid_options"], json!(["Normal", "Numbered"]));
}

#[test]
fn a_style_numbering_nothing_unlinked_is_said_not_changed() {
    // rule 3: a no-op is said, not refused
    let (reply, _) = styles(json!({"doc": "d1", "action": "add", "name": "Grid", "kind": "table"}));
    assert!(reply.get("error").is_none(), "{reply}");
    let (reply, _) = styles(json!({"doc": "d1", "action": "modify", "name": "Normal", "numbering": {"list": "none"}}));
    assert_eq!(
        reply["summary"],
        json!("Modified style Normal: numbers nothing, left as it was"),
        "{reply}"
    );
    assert_eq!(reply["changed"], json!([]));
}

#[test]
fn a_style_is_linked_to_a_list_level_and_the_one_holding_it_said() {
    let (reply, editor) =
        styles(json!({"doc": "d1", "action": "modify", "name": "Normal", "numbering": {"list": "1"}}));
    assert_eq!(
        reply["summary"],
        json!("Modified style Normal: linked to level 0 of list 1"),
        "{reply}"
    );
    assert!(sheet(&editor).contains(r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:style>"#));
}

#[test]
fn the_document_language_is_set_and_reported() {
    let (reply, _) = styles(json!({"doc": "d1", "action": "set_language", "language": "fr_be"}));
    assert_eq!(reply["summary"], json!("The document: language fr-BE"), "{reply}");
    assert_eq!(reply["data"]["default"], json!({"val": "fr-BE"}));
    let (reply, _) = styles(json!({"doc": "d1", "action": "set_language", "language": "fr"}));
    assert_eq!(reply["error"]["field"], json!("language"), "{reply}");
    assert_eq!(reply["error"]["valid_options"][0], json!("fr-FR"));
}

/// A layout whose versions are one page each, and whose reflow says the first page changed
/// and one block moved; counting the reflows asked.
#[derive(Default)]
struct OnePage {
    asked: std::sync::atomic::AtomicUsize,
}

impl FieldLayout for OnePage {
    fn lay_out(&self, _package: &Package, _cancel: &dyn Cancel) -> Result<Box<dyn LaidFields + '_>, LayOutError> {
        Err(LayOutError::Unsupported("not needed".to_owned()))
    }

    fn reflow(&self, before: &Package, after: &Package) -> Result<PageReflow, String> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        assert!(
            before.part_names().len() < after.part_names().len(),
            "the settings part is new"
        );
        Ok(PageReflow {
            changed: vec![1],
            page_count: (1, 2),
            moved: vec![("p:10000003".to_owned(), (1, 2))],
            unknown: Vec::new(),
        })
    }
}

/// `word_template` called on `editor` with `layout`.
fn template(editor: &Editor, layout: Option<&dyn FieldLayout>) -> ToolResult<Value> {
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(editor);
    let editing =
        Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z").with_layout(layout);
    let (args, _) = WordTemplate::accept(json!({"doc": "d1", "action": "upgrade_to_modern"}), Some("docx")).unwrap();
    word_template(&args, &editing)
}

fn settings(editor: &Editor) -> String {
    let head = editor.store().snapshot();
    let bytes = head.doc().package().read("word/settings.xml").unwrap().unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn convert_writes_mode_fifteen_and_reports_the_reflow_then_is_a_no_op() {
    let editor = Editor::open(package(), StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let layout = OnePage::default();
    let reply = Value::Object(template(&editor, Some(&layout)).to_json());
    assert_eq!(reply["summary"], json!("Compatibility mode 15"), "{reply}");
    assert_eq!(
        reply["data"],
        json!({"mode": 15, "reflow": {"changed": [1], "pages": [1, 2], "moved": {"p:10000003": [1, 2]}}})
    );
    assert!(settings(&editor).ends_with(concat!(
        r#"<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/>"#,
        r#"<w:compatSetting w:name="overrideTableStyleFontSizeAndJustification" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/>"#,
        r#"<w:compatSetting w:name="enableOpenTypeFeatures" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/>"#,
        r#"<w:compatSetting w:name="doNotFlipMirrorIndents" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/>"#,
        r#"<w:compatSetting w:name="differentiateMultirowTableHeaders" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/>"#,
        r#"<w:compatSetting w:name="useWord2013TrackBottomHyphenation" w:uri="http://schemas.microsoft.com/office/word" w:val="0"/>"#,
        "</w:compat></w:settings>"
    )));
    // rule 3: a document already in mode 15 is said, not refused, and not laid out
    let reply = Value::Object(template(&editor, Some(&layout)).to_json());
    assert_eq!(reply["summary"], json!("Compatibility mode 15"), "{reply}");
    assert_eq!(reply["data"], json!({"mode": 15, "reflow": null}));
    assert_eq!(reply["changed"], json!([]));
    assert_eq!(layout.asked.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn convert_without_the_layout_is_left_to_the_host() {
    let editor = Editor::open(package(), StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let result = template(&editor, None);
    assert_eq!(result.unported(), ["the reflow without the layout"]);
    assert!(
        editor
            .store()
            .snapshot()
            .doc()
            .package()
            .read("word/settings.xml")
            .unwrap()
            .is_none(),
        "nothing committed"
    );
}

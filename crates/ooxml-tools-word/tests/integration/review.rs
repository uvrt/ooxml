//! `word_set_tracking`, `word_changes` and `word_comments` on made-up documents: their
//! definitions (the golden exports docx-agent's Python test holds to today's), listing,
//! accepting and rejecting, a preview that changes nothing, Word's switch, comments
//! resolved and deleted, and refusals with their way out (tool rules 1-4). Equality with
//! docx-agent on fixtures and the corpus is `crates/ooxml-oracle/python/word_edits_diff.py`'s
//! (`--review`, `--tools`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited, R};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolResult;
use ooxml_tools_word::{
    word_changes, word_comments, word_set_tracking, CallEditor, ChangeAction, ChangeDetail, CommentAction, CommentItem,
    Editing, Tracked, WordChanges, WordComments, WordSetTracking,
};
use ooxml_word_edit::{Editor, WordDoc};
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::Value;
use std::sync::Arc;

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml""#;

/// A document whose body is `body`, with settings and, when given, a comments part and
/// its `commentsExtended`.
fn package(body: &str, comments: Option<(&str, &str)>) -> Package {
    let mut rels = format!(r#"<Relationship Id="rId1" Type="{R}settings" Target="settings.xml"/>"#);
    let mut parts = vec![
        (
            "[Content_Types].xml".to_owned(),
            concat!(
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
                r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
                r#"<Default Extension="xml" ContentType="application/xml"/>"#,
                r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
                r#"</Types>"#
            )
            .to_owned(),
        ),
        (
            "_rels/.rels".to_owned(),
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        (
            "word/settings.xml".to_owned(),
            format!(r#"<w:settings {W}><w:zoom w:percent="100"/><w:defaultTabStop w:val="720"/></w:settings>"#),
        ),
        (
            "word/document.xml".to_owned(),
            format!("<w:document {W}><w:body>{body}</w:body></w:document>"),
        ),
    ];
    if let Some((comments, extended)) = comments {
        rels.push_str(&format!(r#"<Relationship Id="rId2" Type="{R}comments" Target="comments.xml"/><Relationship Id="rId3" Type="http://schemas.microsoft.com/office/2011/relationships/commentsExtended" Target="commentsExtended.xml"/>"#));
        parts.push((
            "word/comments.xml".to_owned(),
            format!("<w:comments {W}>{comments}</w:comments>"),
        ));
        parts.push((
            "word/commentsExtended.xml".to_owned(),
            format!("<w15:commentsEx {W}>{extended}</w15:commentsEx>"),
        ));
    }
    parts.push((
        "word/_rels/document.xml.rels".to_owned(),
        format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#),
    ));
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

/// A tracked body: a replacement by A, an insertion by B.
fn tracked() -> Package {
    let body = concat!(
        r#"<w:p w14:paraId="10000001"><w:r><w:t xml:space="preserve">Keep </w:t></w:r>"#,
        r#"<w:del w:id="1" w:author="A" w:date="2026-01-01T00:00:00Z"><w:r><w:delText>old</w:delText></w:r></w:del>"#,
        r#"<w:ins w:id="2" w:author="A" w:date="2026-01-01T00:00:00Z"><w:r><w:t>new</w:t></w:r></w:ins></w:p>"#,
        r#"<w:p w14:paraId="10000002"><w:r><w:t xml:space="preserve">More </w:t></w:r>"#,
        r#"<w:ins w:id="3" w:author="B" w:date="2026-01-02T00:00:00Z"><w:r><w:t>added</w:t></w:r></w:ins></w:p>"#,
    );
    package(body, None)
}

fn editor(package: Package) -> (Editor, WordDoc) {
    let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    (editor, opened)
}

fn editing<'a>(editor: &'a CallEditor<'a>, opened: &'a WordDoc, caller: &'a Caller) -> Editing<'a> {
    Editing {
        editor,
        caller,
        opened,
        tracking: None,
        doc: "d1",
        cancel: &Never,
        date: "2026-11-17T10:00:00Z",
        layout: None,
        others: ooxml_tools_word::Others::default(),
        blobs: None,
        rows_written: None,
    }
}

fn changes(action: ChangeAction) -> WordChanges {
    WordChanges {
        doc: "d1".into(),
        action,
        ids: None,
        author: None,
        kind: None,
        within: None,
        all: None,
        detail: None,
        cursor: None,
    }
}

fn json(result: &ToolResult<Value>) -> Value {
    Value::Object(result.to_json())
}

#[test]
fn changes_are_listed_grouped_and_as_records() {
    let (editor, opened) = editor(tracked());
    let caller = Caller::anonymous();
    let result = json(&word_changes(
        &changes(ChangeAction::List),
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(result["summary"], "2 change(s)");
    assert_eq!(result["data"][0]["kind"], "replacement");
    assert_eq!(result["data"][0]["old_text"], "old");
    assert_eq!(result["data"][0]["records"], serde_json::json!(["rev:1", "rev:2"]));
    let mut records = changes(ChangeAction::List);
    records.detail = Some(ChangeDetail::Records);
    let result = json(&word_changes(
        &records,
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(result["summary"], "3 record(s)");
    assert_eq!(result["data"][2]["paragraph"], "p:10000002");
}

#[test]
fn accepting_by_author_settles_only_theirs_and_says_what_is_left() {
    let (editor, opened) = editor(tracked());
    let caller = Caller::anonymous();
    let mut call = changes(ChangeAction::Accept);
    call.author = Some("A".into());
    let result = json(&word_changes(
        &call,
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(result["summary"], "Accepted 2 revision record(s)");
    assert_eq!(result["data"]["changes_left"], 1);
    assert_eq!(result["data"]["left_by_author"], serde_json::json!({"B": 1}));
    let left = json(&word_changes(
        &changes(ChangeAction::List),
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(left["data"][0]["author"], "B");
}

#[test]
fn a_preview_changes_nothing() {
    let (editor, opened) = editor(tracked());
    let caller = Caller::anonymous();
    let mut call = changes(ChangeAction::Preview);
    call.all = Some(true);
    let before = editor.store().snapshot().seq();
    let result = json(&word_changes(
        &call,
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert!(result["summary"]
        .as_str()
        .unwrap()
        .starts_with("Preview, nothing changed: 2 change(s); accept would settle 3 record(s)"));
    assert_eq!(result["data"]["reject"]["changes_left"], 0);
    assert_eq!(editor.store().snapshot().seq(), before);
}

#[test]
fn refusals_carry_the_way_out() {
    let (editor, opened) = editor(tracked());
    let caller = Caller::anonymous();
    let result = json(&word_changes(
        &changes(ChangeAction::Accept),
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(result["error"]["code"], "invalid_arguments");
    assert!(result["error"]["valid_options"]
        .as_array()
        .is_some_and(|o| o.len() == 5));
    let mut call = changes(ChangeAction::Reject);
    call.ids = Some(vec!["rev:99".into()]);
    let result = json(&word_changes(
        &call,
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(result["error"]["code"], "not_found");
    assert!(result["error"]["valid_options"]
        .as_array()
        .is_some_and(|o| !o.is_empty()));
    let mut call = changes(ChangeAction::List);
    call.all = Some(true);
    let result = json(&word_changes(
        &call,
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(result["error"]["field"], "all");
}

#[test]
fn tracking_is_handed_back_and_words_switch_set_once() {
    let (editor, opened) = editor(tracked());
    let caller = Caller::anonymous();
    let call = WordSetTracking {
        doc: "d1".into(),
        on: true,
        author: Some("Reviewer".into()),
        word_switch: Some(true),
    };
    let (result, mode) = word_set_tracking(&call, &editing(&CallEditor::new(&editor), &opened, &caller));
    assert_eq!(mode, Some(Tracked::On("Reviewer".into())));
    assert_eq!(json(&result)["data"]["word_switch"], true);
    let seq = editor.store().snapshot().seq();
    let (again, _) = word_set_tracking(&call, &editing(&CallEditor::new(&editor), &opened, &caller));
    assert_eq!(json(&again)["summary"], "Tracking on, as Reviewer; Word's switch on");
    assert_eq!(
        editor.store().snapshot().seq(),
        seq,
        "a switch already on is not written again (rule 3)"
    );
    let missing = WordSetTracking {
        author: None,
        word_switch: None,
        ..call
    };
    let (refused, mode) = word_set_tracking(&missing, &editing(&CallEditor::new(&editor), &opened, &caller));
    assert_eq!(mode, None);
    assert_eq!(json(&refused)["error"]["field"], "author");
}

#[test]
fn comments_are_resolved_and_deleted_with_their_replies() {
    let body = concat!(
        r#"<w:p w14:paraId="10000001"><w:commentRangeStart w:id="0"/><w:r><w:t>Text</w:t></w:r>"#,
        r#"<w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r>"#,
        r#"<w:r><w:commentReference w:id="1"/></w:r></w:p>"#
    );
    let comments = concat!(
        r#"<w:comment w:id="0" w:author="A"><w:p w14:paraId="20000001"><w:r><w:t>Look</w:t></w:r></w:p></w:comment>"#,
        r#"<w:comment w:id="1" w:author="B"><w:p w14:paraId="20000002"><w:r><w:t>Agreed</w:t></w:r></w:p></w:comment>"#
    );
    let extended = r#"<w15:commentEx w15:paraId="20000001" w15:done="0"/><w15:commentEx w15:paraId="20000002" w15:paraIdParent="20000001" w15:done="0"/>"#;
    let (editor, opened) = editor(package(body, Some((comments, extended))));
    let caller = Caller::anonymous();
    let item = |id: &str| CommentItem {
        target: None,
        find: None,
        within: None,
        comment: Some(id.into()),
        text: None,
        resolve: None,
        ref_name: None,
    };
    let call = |action, items| WordComments {
        doc: "d1".into(),
        action,
        items,
        author: None,
        open_only: None,
        cursor: None,
    };
    let listed = json(&word_comments(
        &call(CommentAction::List, None),
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(listed["summary"], "1 thread(s)");
    assert_eq!(listed["data"][0]["replies"][0]["text"], "Agreed");
    let resolved = json(&word_comments(
        &call(CommentAction::Resolve, Some(vec![item("c#1")])),
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(
        resolved["changed"],
        serde_json::json!(["c#0"]),
        "a reply resolves its thread"
    );
    let deleted = json(&word_comments(
        &call(CommentAction::Delete, Some(vec![item("c#0")])),
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(deleted["removed"], serde_json::json!(["c#0", "c#1"]));
    let empty = json(&word_comments(
        &call(CommentAction::Delete, Some(Vec::new())),
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(
        empty["summary"], "Nothing changed: items is empty",
        "an empty list is no error (rule 3)"
    );
}

/// An item of `add` (on `target`, ref `q`) or `reply` (to `comment`).
fn new_item(target: Option<&str>, comment: Option<&str>, text: &str, resolve: Option<bool>) -> CommentItem {
    CommentItem {
        target: target.map(Into::into),
        find: None,
        within: None,
        comment: comment.map(Into::into),
        text: Some(text.into()),
        resolve,
        ref_name: target.map(|_| "q".into()),
    }
}

#[test]
fn comments_are_added_and_replied_to_with_their_parts() {
    let body = r#"<w:p w14:paraId="10000001"><w:r><w:t>Some text here</w:t></w:r></w:p>"#;
    let (editor, opened) = editor(package(body, None));
    let caller = Caller::anonymous();
    let call = |action, items| WordComments {
        doc: "d1".into(),
        action,
        items: Some(items),
        author: Some("Reviewer".into()),
        open_only: None,
        cursor: None,
    };
    let added = json(&word_comments(
        &call(
            CommentAction::Add,
            vec![new_item(Some("p:10000001@5:9"), None, "Why?", None)],
        ),
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    let id = added["data"]["comments"][0]
        .as_str()
        .unwrap_or_else(|| panic!("{added}"))
        .to_owned();
    assert!(id.starts_with("c:"), "{added}");
    assert_eq!(added["created"], serde_json::json!([id]));
    let head = editor.store().snapshot();
    let package = head.doc().package();
    for part in [
        "word/comments.xml",
        "word/commentsExtended.xml",
        "word/commentsIds.xml",
        "word/commentsExtensible.xml",
        "word/people.xml",
    ] {
        assert!(package.has_part(part), "{part} made");
    }
    let document = String::from_utf8(package.read("word/document.xml").unwrap().unwrap().to_vec()).unwrap();
    assert!(
        document.contains(r#"<w:t xml:space="preserve">Some </w:t></w:r><w:commentRangeStart w:id="0"/>"#),
        "the range starts at the fifth character: {document}"
    );
    let replied = json(&word_comments(
        &call(
            CommentAction::Reply,
            vec![new_item(None, Some(&id), "Because.", Some(true))],
        ),
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(
        replied["changed"],
        serde_json::json!([id]),
        "the thread resolved: {replied}"
    );
    let listed = json(&word_comments(
        &WordComments {
            doc: "d1".into(),
            action: CommentAction::List,
            items: None,
            author: None,
            open_only: None,
            cursor: None,
        },
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(listed["data"][0]["done"], true);
    assert_eq!(listed["data"][0]["on"]["text"], "text");
    assert_eq!(listed["data"][0]["replies"][0]["text"], "Because.");
    assert_eq!(listed["data"][0]["author"], "Reviewer");
}

#[test]
fn text_is_found_within_the_paragraphs_an_address_holds() {
    let body = concat!(
        r#"<w:p w14:paraId="10000001"><w:r><w:t>A note on cats</w:t></w:r></w:p>"#,
        r#"<w:p w14:paraId="10000002"><w:r><w:t>More on cats</w:t></w:r></w:p>"#
    );
    let (editor, opened) = editor(package(body, None));
    let caller = Caller::anonymous();
    let add = |find: &str, within: &str| WordComments {
        doc: "d1".into(),
        action: CommentAction::Add,
        items: Some(vec![CommentItem {
            target: None,
            find: Some(find.into()),
            within: Some(within.into()),
            comment: None,
            text: Some("Why?".into()),
            resolve: None,
            ref_name: None,
        }]),
        author: None,
        open_only: None,
        cursor: None,
    };
    // "cats" occurs twice in the body, once in the second paragraph (a range's whole paragraph)
    let added = json(&word_comments(
        &add("cats", "p:10000002@0:4"),
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert!(added["data"]["comments"][0].is_string(), "{added}");
    let listed = json(&word_comments(
        &WordComments {
            doc: "d1".into(),
            action: CommentAction::List,
            items: None,
            author: None,
            open_only: None,
            cursor: None,
        },
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert_eq!(listed["data"][0]["on"]["text"], "cats", "{listed}");
    let nowhere = json(&word_comments(
        &add("note", "p:10000002"),
        &editing(&CallEditor::new(&editor), &opened, &caller),
    ));
    assert!(
        nowhere.to_string().contains("'note' occurs nowhere in p:10000002"),
        "{nowhere}"
    );
}

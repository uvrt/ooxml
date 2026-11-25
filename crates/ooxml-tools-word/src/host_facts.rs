//! Owns: what a host holds of the Word tools beside their specs (docx-agent's `HOST` tables
//! and its format's hints and withdrawn tools, as data): the roles of their actions, the role
//! a comment reply that resolves its thread needs, the calls of a changing tool that only
//! read, a reviewer's main tools ([`host_facts`]); the names a model writes that the host's
//! reading of names does not settle ([`TOOL_HINTS`]); and the tools withdrawn until the
//! session has them again ([`WITHDRAWN`]).
//!
//! Must not: change a spec or its export (these are the host's facts, not the definitions
//! the model is sent), or run a tool.

use ooxml_tools::roles::{HostFacts, Needs, Role};
use serde_json::{Map, Value};

/// Names a model writes, each with the tool (and action) it means.
pub const TOOL_HINTS: [(&str, &str); 31] = [
    ("word_add_image", "word_drawings insert_picture"),
    ("word_add_textbox", "word_drawings insert_text_box"),
    ("word_add_table", "word_insert_markdown"),
    ("word_add_heading", "word_insert_markdown"),
    ("word_add_paragraph", "word_insert_markdown"),
    ("word_add_bullet", "word_lists add"),
    ("word_add_link", "word_links add_hyperlink"),
    ("word_add_url", "word_links add_hyperlink"),
    ("word_add_page_break", "word_format"),
    ("word_set_font", "word_format"),
    ("word_apply_style", "word_format"),
    ("word_set_paragraph_style", "word_format"),
    ("word_format_cell", "word_format_table"),
    ("word_shade_cell", "word_format_table"),
    ("word_track_changes", "word_set_tracking"),
    ("word_enable_tracking", "word_set_tracking"),
    ("word_add_header", "word_headers_footers set"),
    ("word_add_footer", "word_headers_footers set"),
    ("word_add_page_number", "word_headers_footers set"),
    ("word_set_margins", "word_sections set"),
    ("word_set_orientation", "word_sections set"),
    ("word_set_page_size", "word_sections set"),
    ("word_set_columns", "word_sections set"),
    ("word_add_checkbox", "word_controls insert"),
    ("word_add_dropdown", "word_controls insert"),
    ("word_add_content_control", "word_controls insert"),
    ("word_find", "find_text"),
    ("word_replace", "replace_text"),
    ("word_find_replace", "replace_text"),
    ("word_save", "save_document"),
    ("word_upgrade", "word_template upgrade_to_modern"),
];

/// The tools withdrawn until the session has them again, each with what to call instead.
pub const WITHDRAWN: [(&str, &str); 3] = [
    (
        "edit_smartart",
        "SmartArt is kept as it is; word_drawings inserts a text box or a shape (insert_text_box, insert_shape) \
         to draw the diagram's content instead.",
    ),
    (
        "style_by_value",
        "check with scale lists the values with the wrong step or none; style each with word_format_table \
         (shading, for a cell) or word_format (color).",
    ),
    (
        "word_find_asset",
        "give the picture as an image blob: word_drawings with action insert_picture and image.",
    ),
];

const REVIEW: Needs = Needs::Role(Role::Review);
const RESOLVE: Needs = Needs::Role(Role::Resolve);
const ACCEPT: Needs = Needs::Role(Role::Accept);

fn action(arguments: &Map<String, Value>) -> Option<&str> {
    arguments.get("action").and_then(Value::as_str)
}

/// `word_changes`' calls that only read: `list` and `preview`.
fn listing_changes(arguments: &Map<String, Value>) -> bool {
    matches!(action(arguments), Some("list" | "preview"))
}

/// `word_follow_sample`'s calls that only read: a `preview`.
fn previewing(arguments: &Map<String, Value>) -> bool {
    arguments.get("preview").is_some_and(truthy)
}

/// `edit_chart`'s calls that only read: `read`.
fn reading_chart(arguments: &Map<String, Value>) -> bool {
    action(arguments) == Some("read")
}

/// A comment reply that also resolves its thread (an item's `resolve: true`) needs `resolve`.
fn comment_role(arguments: &Map<String, Value>) -> Option<Needs> {
    let resolves = |items: &Vec<Value>| {
        items
            .iter()
            .any(|item| item.as_object().and_then(|i| i.get("resolve")).is_some_and(truthy))
    };
    match (action(arguments), arguments.get("items")) {
        (Some("reply"), Some(Value::Array(items))) if resolves(items) => Some(RESOLVE),
        _ => None,
    }
}

/// A value as Python's `bool` reads it.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn roles(pairs: &[(&str, Needs)]) -> Vec<(String, Needs)> {
    pairs.iter().map(|(a, n)| ((*a).to_owned(), *n)).collect()
}

/// What a host holds of each Word tool beside its spec, by tool name.
#[must_use]
pub fn host_facts() -> Vec<(String, HostFacts)> {
    let listing = || HostFacts {
        action_roles: roles(&[("list", REVIEW)]),
        ..HostFacts::default()
    };
    let mut out = vec![
        (
            "edit_chart".to_owned(),
            HostFacts {
                action_roles: roles(&[("read", REVIEW)]),
                reads: Some(reading_chart),
                ..HostFacts::default()
            },
        ),
        (
            "word_changes".to_owned(),
            HostFacts {
                action_roles: roles(&[
                    ("list", REVIEW),
                    ("preview", ACCEPT),
                    ("accept", ACCEPT),
                    ("reject", ACCEPT),
                ]),
                reads: Some(listing_changes),
                review_core: true,
                action_fields: ["ids", "all"]
                    .iter()
                    .map(|f| {
                        (
                            (*f).to_owned(),
                            vec!["accept".to_owned(), "reject".to_owned(), "preview".to_owned()],
                        )
                    })
                    .collect(),
                ..HostFacts::default()
            },
        ),
        (
            "word_comments".to_owned(),
            HostFacts {
                action_roles: roles(&[
                    ("list", REVIEW),
                    ("add", REVIEW),
                    ("reply", REVIEW),
                    ("resolve", RESOLVE),
                    ("reopen", RESOLVE),
                    ("delete", RESOLVE),
                    ("edit", Needs::Any),
                ]),
                call_role: Some(comment_role),
                review_core: true,
                ..HostFacts::default()
            },
        ),
        (
            "word_styles".to_owned(),
            HostFacts {
                action_roles: roles(&[("list", REVIEW), ("describe", REVIEW)]),
                ..HostFacts::default()
            },
        ),
        (
            "word_follow_sample".to_owned(),
            HostFacts {
                reads: Some(previewing),
                ..HostFacts::default()
            },
        ),
    ];
    for name in [
        "word_sections",
        "word_fields",
        "word_notes",
        "word_links",
        "word_drawings",
        "word_controls",
    ] {
        out.push((name.to_owned(), listing()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_reply_that_resolves_its_thread_needs_resolve() {
        let call = |v: Value| v.as_object().cloned().unwrap_or_default();
        assert_eq!(
            comment_role(&call(json!({"action": "reply", "items": [{"resolve": true}]}))),
            Some(RESOLVE)
        );
        assert_eq!(
            comment_role(&call(json!({"action": "reply", "items": [{"text": "ok"}]}))),
            None
        );
        assert!(previewing(&call(json!({"preview": true}))));
        assert!(!previewing(&call(json!({"preview": false}))));
        assert!(listing_changes(&call(json!({"action": "preview"}))));
    }
}

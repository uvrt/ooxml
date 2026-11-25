//! Owns: `word_links` (docx-agent's `tools/structure.py`, `word_links`): its definition,
//! its list, and its edits through the edit core's link commands.
//!
//! Must not: change the document but through a command.

use super::common::{call, find_once, need, one_of, outcome, policy, refused};
use crate::describe::short;
use crate::review::Editing;
use ooxml_tools::{ArgEnum, Done, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{hyperlink_span, AddBookmark, AddHyperlink, RemoveBookmark, RenameBookmark};
use ooxml_word_model::address::marks::{bookmark_markers, bookmark_range, range_text};
use ooxml_word_model::docx::text::{paragraph_text, View};
use ooxml_word_model::ids::W;
use serde_json::{json, Value};
use std::sync::Arc;

const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// What `word_links` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkAction {
    /// List bookmarks and hyperlinks.
    List,
    /// Make text a hyperlink.
    AddHyperlink,
    /// Bookmark text or blocks.
    AddBookmark,
    /// Rename a bookmark and what points at it.
    RenameBookmark,
    /// Remove a bookmark's markers.
    RemoveBookmark,
}

/// `word_links`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(name = "word_links", description = include_str!("word_links.md"), group = "word_structure", mutates)]
pub struct WordLinks {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: LinkAction,
    /// add_*: range or paragraph; add_bookmark: or blocks (t:X, p:A..t:B).
    #[arg(refs)]
    pub target: Option<String>,
    /// add_*: or text occurring once.
    pub find: Option<String>,
    /// add_hyperlink: external address.
    pub url: Option<String>,
    /// Bookmark name; add_hyperlink: link to it instead of url.
    pub bookmark: Option<String>,
    /// rename_bookmark: new name.
    pub new_name: Option<String>,
}

/// `word_links` (docx-agent's `word_links`): list the bookmarks and hyperlinks; make text a
/// hyperlink; bookmark text or blocks; rename or remove a bookmark -- each one command.
#[must_use]
pub fn word_links(args: &WordLinks, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let list = call("word_links", &[("doc", json!(args.doc)), ("action", json!("list"))]);
    match run(args, editing, &list) {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

fn run(args: &WordLinks, editing: &Editing<'_>, list: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let editor = editing.editor;
    let done = match args.action {
        LinkAction::List => return Ok(listed(editing)),
        LinkAction::RemoveBookmark => {
            let name = need(args.bookmark.as_deref(), "bookmark", list)?;
            let command = RemoveBookmark { name: name.to_owned() };
            match editor.run(editing.caller, &command) {
                Ok(done) => outcome(&[done.out.into()], format!("Removed bookmark {name}"), None),
                Err(error) => refused(error, list.clone()),
            }
        }
        LinkAction::RenameBookmark => {
            let name = need(args.bookmark.as_deref(), "bookmark", list)?;
            let new = need(args.new_name.as_deref(), "new_name", list)?;
            let command = RenameBookmark {
                name: name.to_owned(),
                new: new.to_owned(),
            };
            match editor.run(editing.caller, &command) {
                Ok(done) => outcome(&[done.out.into()], format!("Renamed bookmark {name} to {new}"), None),
                Err(error) => refused(error, list.clone()),
            }
        }
        LinkAction::AddBookmark | LinkAction::AddHyperlink => {
            let found = one_of(
                &[("target", args.target.is_some()), ("find", args.find.is_some())],
                list,
            )? == "find";
            let place = match (&args.find, &args.target) {
                (Some(text), _) if found => find_once(editing, text, list)?,
                (_, Some(target)) => target.clone(),
                _ => String::new(),
            };
            if args.action == LinkAction::AddBookmark {
                let name = need(args.bookmark.as_deref(), "bookmark", list)?;
                let command = AddBookmark {
                    place,
                    name: name.to_owned(),
                };
                match editor.run(editing.caller, &command) {
                    Ok(done) => {
                        let summary = if done.out.changed {
                            format!("Bookmarked as {name}")
                        } else {
                            format!("No change: {}", done.out.warnings.first().map_or("", String::as_str))
                        };
                        outcome(&[done.out.into()], summary, None)
                    }
                    Err(error) => refused(error, list.clone()),
                }
            } else {
                if args.url.is_none() == args.bookmark.is_none() {
                    let way_out = WayOut::names(["url", "bookmark"]).unwrap_or_else(|| WayOut::call(list.clone()));
                    return Err(ToolError::refusal(
                        Refusal::InvalidArguments,
                        "give exactly one of url, bookmark",
                        way_out,
                    ));
                }
                let command = AddHyperlink {
                    range: place.into(),
                    url: args.url.clone(),
                    anchor: args.bookmark.clone(),
                    policy: policy(editing),
                };
                match editor.run(editing.caller, &command) {
                    Ok(done) => outcome(&[done.out.into()], "Made a hyperlink".to_owned(), None),
                    Err(error) => refused(error, list.clone()),
                }
            }
        }
    };
    Ok(done)
}

/// The bookmarks (Word's hidden ones left out) and hyperlinks, a hundred of each, as
/// docx-agent lists them.
fn listed(editing: &Editing<'_>) -> ToolResult<Value> {
    let head = editing.editor.current();
    let docx = match head.doc().docx() {
        Ok(docx) => docx,
        Err(error) => return ToolResult::failed(ToolError::internal(error.to_string(), "word:model")),
    };
    let bookmarks: Vec<Value> = bookmark_markers(&docx)
        .iter()
        .filter(|b| !b.name.starts_with('_'))
        .take(100)
        .map(|b| {
            let text = bookmark_range(&docx, b)
                .map(|r| range_text(b.part, &r, View::Current))
                .unwrap_or_default();
            json!({"name": b.name, "text": short(&text, 80)})
        })
        .collect();
    let mut hyperlinks = Vec::new();
    'parts: for story in &docx.parts {
        let relationships = head.doc().package().relationships(&story.part).ok();
        for paragraph in story.ids.paragraphs().iter() {
            let links: Vec<&Arc<ooxml_xml::Element>> = descendants(&paragraph.element, "hyperlink");
            if links.is_empty() {
                continue;
            }
            let text: Vec<char> = paragraph_text(&paragraph.element, View::Current).chars().collect();
            for (k, link) in links.into_iter().enumerate() {
                if hyperlinks.len() == 100 {
                    break 'parts;
                }
                let (a, b) = hyperlink_span(&paragraph.element, k).unwrap_or((0, 0));
                let said: String = text.get(a..b).unwrap_or_default().iter().collect();
                let url = link.attribute(Some(R), "id").and_then(|id| {
                    relationships
                        .as_ref()?
                        .iter()
                        .find(|r| r.id == id)
                        .map(|r| r.target.clone())
                });
                hyperlinks.push(json!({
                    "id": format!("hl:{}/{k}", paragraph.id),
                    "text": short(&said, 80),
                    "url": url,
                    "bookmark": link.attribute(Some(W), "anchor"),
                }));
            }
        }
    }
    let mut done = Done::new("Links and bookmarks");
    done.data = Some(json!({"bookmarks": bookmarks, "hyperlinks": hyperlinks}));
    ToolResult::done(done)
}

/// Every element `local` (Word's) below `element`, in document order (lxml's `iter`).
fn descendants<'e>(element: &'e Arc<ooxml_xml::Element>, local: &str) -> Vec<&'e Arc<ooxml_xml::Element>> {
    let mut out = Vec::new();
    for node in element.children().iter() {
        if let ooxml_xml::Node::Element(child) = node {
            if child.name().is(Some(W), local) {
                out.push(child);
            }
            out.extend(descendants(child, local));
        }
    }
    out
}

/// Registers `word_links` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordLinks>(word_links, crate::Hosting::PLAIN);
}

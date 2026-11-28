//! Owns: comments as docx-agent reads them (`edit/annotations.py`, `_comment_records`): each
//! `w:comment` of the main part's comments part with its id (`c:<durableId>` where
//! `commentsIds` names one for its last paragraph, else `c#<w:id>`), author, whether it is
//! resolved (`commentsExtended`), the comment it replies to, and its text.
//!
//! Must not: change a comment.

use crate::docx::text::{paragraph_text, OBJECT};

use crate::docx::Docx;
use crate::ids::{W, W14};
use crate::parts::{related, related_parts, REL_COMMENTS, REL_COMMENTS_EXTENDED, REL_COMMENTS_IDS, W15, W16CID};
use crate::view::View;
use ooxml_xml::Node;
use std::collections::HashMap;

/// A comment.
#[derive(Debug, Clone)]
pub struct Comment {
    /// Its id.
    pub id: String,
    /// Its `w:id`.
    pub w_id: Option<String>,
    /// Its author.
    pub author: Option<String>,
    /// Whether it is resolved.
    pub done: bool,
    /// The id of the comment it replies to.
    pub parent: Option<String>,
    /// Its paragraphs' text in the current view, objects left out, one space between.
    pub text: String,
    /// What it says: its paragraphs' text in the current view joined by line ends, objects
    /// left out, trimmed (docx-agent's `Comment.text`).
    pub says: String,
    /// `w:initials`.
    pub initials: Option<String>,
    /// `w:date`.
    pub date: Option<String>,
    /// Its part.
    pub part: String,
    /// Its place in its part.
    pub path: Box<[u32]>,
    /// The `w14:paraId` of its last paragraph, upper case (empty: none).
    pub last_para: String,
}

/// A `commentsIds` or `commentsEx` entry: its `durableId`, `paraIdParent` and `done`.
type Entry = (Option<String>, Option<String>, Option<String>);

fn by_para(docx: &Docx, rel_type: &str, namespace: &str) -> HashMap<String, Entry> {
    let mut out = HashMap::new();
    let Ok(Some(part)) = related(&docx.package, &docx.main, rel_type) else {
        return out;
    };
    let Ok(Some(tree)) = docx.package.tree(&part) else {
        return out;
    };
    for node in tree.root().child_elements() {
        let para = node.attribute(Some(namespace), "paraId").unwrap_or("").to_uppercase();
        let get = |local: &str| node.attribute(Some(namespace), local).map(str::to_owned);
        out.insert(para, (get("durableId"), get("paraIdParent"), get("done")));
    }
    out
}

/// Every comment, in document order.
#[must_use]
pub fn comments(docx: &Docx) -> Vec<Comment> {
    let Some(part) = related_parts(&docx.package, &docx.main, REL_COMMENTS)
        .ok()
        .and_then(|p| p.into_iter().next())
        .and_then(|p| docx.part(&p))
    else {
        return Vec::new();
    };
    let durable = by_para(docx, REL_COMMENTS_IDS, W16CID);
    let extended = by_para(docx, REL_COMMENTS_EXTENDED, W15);
    let mut out: Vec<(Comment, Option<String>)> = Vec::new();
    let mut by_last: HashMap<String, String> = HashMap::new();
    for (k, node) in part.root().children().iter().enumerate() {
        let Node::Element(comment) = node else { continue };
        if !comment.name().is(Some(W), "comment") {
            continue;
        }
        let path = [u32::try_from(k).unwrap_or(u32::MAX)];
        let inside: Vec<crate::ids::ParagraphId> = part.ids.paragraphs_within(&path);
        let last = last_para(comment);
        let id = match durable.get(&last).and_then(|d| d.0.clone()).filter(|d| !d.is_empty()) {
            Some(d) => format!("c:{d}"),
            None => format!("c#{}", comment.attribute(Some(W), "id").unwrap_or("None")),
        };
        let (parent_para, done) = extended.get(&last).map_or((None, false), |e| {
            let parent = e.1.as_deref().map(str::to_uppercase).filter(|p| !p.is_empty());
            (parent, matches!(e.2.as_deref().unwrap_or("0"), "1" | "true"))
        });
        let says = inside
            .iter()
            .map(|p| paragraph_text(&p.element, View::Final))
            .collect::<Vec<_>>()
            .join("\n");
        let texts: Vec<String> = inside
            .iter()
            .map(|p| {
                paragraph_text(&p.element, View::Final)
                    .replace(OBJECT, "")
                    .trim()
                    .to_owned()
            })
            .filter(|t| !t.is_empty())
            .collect();
        if !last.is_empty() {
            by_last.insert(last.clone(), id.clone());
        }
        out.push((
            Comment {
                id,
                w_id: comment.attribute(Some(W), "id").map(str::to_owned),
                author: comment.attribute(Some(W), "author").map(str::to_owned),
                done,
                parent: None,
                text: texts.join(" "),
                says: says.replace(OBJECT, "").trim().to_owned(),
                initials: comment.attribute(Some(W), "initials").map(str::to_owned),
                date: comment.attribute(Some(W), "date").map(str::to_owned),
                part: part.part.clone(),
                path: Box::new(path),
                last_para: last.clone(),
            },
            parent_para,
        ));
    }
    out.into_iter()
        .map(|(mut c, parent)| {
            c.parent = parent.and_then(|p| by_last.get(&p).cloned());
            c
        })
        .collect()
}

/// The `w14:paraId` of a comment's last paragraph (any depth), upper case.
fn last_para(comment: &ooxml_xml::Element) -> String {
    let mut last = String::new();
    let mut stack = vec![comment.children().iter()];
    while let Some(top) = stack.last_mut() {
        match top.next() {
            Some(Node::Element(e)) => {
                if e.name().is(Some(W), "p") {
                    last = e.attribute(Some(W14), "paraId").unwrap_or("").to_uppercase();
                }
                stack.push(e.children().iter());
            }
            Some(_) => {}
            None => {
                stack.pop();
            }
        }
    }
    last
}

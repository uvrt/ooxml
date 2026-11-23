//! Owns: a page's SVG tagged with the tool's ids of the version it draws: every paragraph
//! group (`<g data-docx-path=... [data-docx-id=...] [data-docx-story=... data-docx-part=...]>`)
//! given `data-docx-agent-id`, the id of the paragraph its path names in its part -- the
//! story's part a header's or footer's group names, the notes part for a note's path, else
//! the main part -- as docx-agent's `layout._tag` gives it.
//!
//! Must not: change anything of the SVG but add the attribute, or tag a group that is not a
//! paragraph's (a drawing's, a line's).

use ooxml_word_model::docx::{Docx, StoryPart};
use ooxml_word_model::ids::W;
use std::collections::HashMap;

/// What opens a paragraph's group.
const GROUP: &str = "<g data-docx-path=\"";

/// `svg` with `data-docx-agent-id` on every paragraph group whose paragraph `docx` names.
pub(super) fn tagged(svg: &str, docx: &Docx) -> String {
    let mut out = String::with_capacity(svg.len() + svg.len() / 16);
    let mut ids: HashMap<(Option<String>, String), Option<String>> = HashMap::new();
    let mut rest = svg;
    while let Some(at) = rest.find(GROUP) {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        let Some((attributes, end)) = attributes(from) else {
            out.push_str(GROUP);
            rest = from.get(GROUP.len()..).unwrap_or_default();
            continue;
        };
        let head = from.get(..end).unwrap_or_default();
        out.push_str(head);
        if let Some((path, part)) = paragraph_group(&attributes) {
            let found = ids
                .entry((part.clone(), path.clone()))
                .or_insert_with(|| paragraph_id(docx, &path, part.as_deref()));
            if let Some(id) = found {
                out.push_str(" data-docx-agent-id=\"");
                out.push_str(&escape(id));
                out.push('"');
            }
        }
        out.push('>');
        rest = from.get(end + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// The attributes of the start tag at the start of `tag` (`<g name="value" ...>`), values
/// unescaped, and where its `>` is; `None` where it is no plain start tag.
fn attributes(tag: &str) -> Option<(Vec<(&str, String)>, usize)> {
    let mut at = 2; // past `<g`
    let mut out = Vec::new();
    loop {
        let rest = tag.get(at..)?;
        if rest.starts_with('>') {
            return Some((out, at));
        }
        let rest = rest.strip_prefix(' ')?;
        let eq = rest.find("=\"")?;
        let name = rest.get(..eq)?;
        if name.is_empty() || name.contains([' ', '>', '/']) {
            return None;
        }
        let value_from = eq + 2;
        let close = rest.get(value_from..)?.find('"')?;
        out.push((name, unescape(rest.get(value_from..value_from + close)?)));
        at += 1 + value_from + close + 1;
    }
}

/// The path and part (where the group names one) of a paragraph's group: its attributes
/// are its path, perhaps its paragraph id, perhaps its story and part -- nothing else.
fn paragraph_group(attributes: &[(&str, String)]) -> Option<(String, Option<String>)> {
    let names: Vec<&str> = attributes.iter().map(|(n, _)| *n).collect();
    let value = |name: &str| attributes.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone());
    let part = match names.as_slice() {
        ["data-docx-path"] | ["data-docx-path", "data-docx-id"] => None,
        ["data-docx-path", "data-docx-story", "data-docx-part"]
        | ["data-docx-path", "data-docx-id", "data-docx-story", "data-docx-part"] => value("data-docx-part"),
        _ => return None,
    };
    Some((value("data-docx-path")?, part))
}

/// The id of the paragraph `path` names in its part (`docx2svg.paths.path_part`): `part`
/// where the group names it, the footnotes' or endnotes' part for a note's path, else the
/// main part. `None` where the path names no paragraph there.
fn paragraph_id(docx: &Docx, path: &str, part: Option<&str>) -> Option<String> {
    let story = match part {
        Some(part) => named(docx, part)?,
        None => {
            let first = path.split('/').next().unwrap_or_default();
            let note = ["w:footnote", "w:endnote"]
                .into_iter()
                .find(|prefix| first == format!("{prefix}s") || first.starts_with(&format!("{prefix}[")));
            match note {
                Some(prefix) => notes(docx, prefix.trim_start_matches("w:"))?,
                None => docx.body()?,
            }
        }
    };
    let (element, place) = story.resolve_path(path)?;
    if !element.name().is(Some(W), "p") {
        return None;
    }
    story.ids.paragraph_at(&place).map(|p| p.id)
}

/// The story part named `part` (with or without its leading `/`).
fn named<'d>(docx: &'d Docx, part: &str) -> Option<&'d StoryPart> {
    let part = part.trim_start_matches('/');
    docx.parts
        .iter()
        .map(|p| &**p)
        .find(|p| p.part.trim_start_matches('/') == part)
}

/// The notes part whose root is `w:<note>s` (`footnote`, `endnote`).
fn notes<'d>(docx: &'d Docx, note: &str) -> Option<&'d StoryPart> {
    let root = format!("{note}s");
    docx.parts
        .iter()
        .map(|p| &**p)
        .find(|p| p.tree.root().name().is(Some(W), &root))
}

/// An attribute's value as written, its five entities read.
fn unescape(value: &str) -> String {
    if !value.contains('&') {
        return value.to_owned();
    }
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// A value written in an attribute (docx-agent's `_escape`).
fn escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_paragraphs_group_is_read() {
        let tag = r#"<g data-docx-path="w:body/w:p[1]" data-docx-id="1A">x"#;
        let (found, end) = attributes(tag).unwrap();
        assert_eq!(Some(end), tag.find('>'));
        assert_eq!(paragraph_group(&found), Some(("w:body/w:p[1]".to_owned(), None)));
        let (found, _) = attributes(
            r#"<g data-docx-path="w:hdr/w:p[1]" data-docx-story="header" data-docx-part="word/header1.xml">"#,
        )
        .unwrap();
        assert_eq!(
            paragraph_group(&found),
            Some(("w:hdr/w:p[1]".to_owned(), Some("word/header1.xml".to_owned())))
        );
        let (found, _) = attributes(r#"<g data-docx-path="w:body/w:p[1]/w:r[1]" data-docx-float="front">"#).unwrap();
        assert_eq!(paragraph_group(&found), None);
        assert!(attributes(r#"<g data-docx-path="a"/>"#).is_none());
        assert_eq!(unescape("a&amp;b&quot;"), "a&b\"");
    }
}

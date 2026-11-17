//! `describe`: the document at a glance (today's `word_describe`), over package parts.
//!
//! Owns: the tool's definition (one schema for every kind, as the shared `describe` has)
//! and the fields of its data that the parts give without the Word model: the name, the
//! compatibility mode, the core properties, comments, notes and content controls, each as
//! docx-agent counts it, keyed and ordered as its reply is.
//!
//! Must not: lay out (pages), resolve styles (headings, styles in use), give ids from the
//! id ledger (headings, tables, drawings, tagged controls), or read the session's state
//! (tracking, the problems a document opened with): those fields are [`NOT_DESCRIBED`]
//! until the model and the session have them.

use crate::package::{
    descendants, leading_text, live_elements, main_part, related, related_parts, story_parts, REL_COMMENTS,
    REL_COMMENTS_EXTENDED, REL_COMMENTS_IDS, REL_CORE, REL_SETTINGS, W, W14, W15, W16CID,
};
use ooxml_base::{BaseError, Cancel};
use ooxml_opc::{OpcError, Package};
use ooxml_tools::{Done, Failure, ToolArgs, ToolError};
use ooxml_xml::{Element, Tree};
use serde_json::{Map, Value};

/// `describe`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(name = "describe", description = include_str!("describe.md"))]
pub struct Describe {
    /// Document id.
    pub doc: String,
}

/// The keys of today's data that this handler gives, in their order there.
pub const DESCRIBED: &[&str] = &[
    "name",
    "compatibility_mode",
    "properties",
    "comments",
    "notes",
    "controls",
];

/// The keys of today's data that it does not give yet, and why (the hand-back's list).
pub const NOT_DESCRIBED: &[(&str, &str)] = &[
    ("pages", "the Word layout (phase PL, step L4)"),
    ("tracking", "the session's tracking state (phase P2)"),
    ("headings", "the styles cascade and the id ledger (phase P1)"),
    ("headings_note", "with headings"),
    ("sections", "the Word model's sections (phase P1)"),
    ("styles_in_use", "the styles cascade (phase P1)"),
    ("revisions", "tracked changes grouped as changes (phase P1)"),
    ("fields", "fields threaded through runs (phase P1)"),
    ("tables", "the id ledger and accepted sizes (phase P1)"),
    ("drawings", "the id ledger (phase P1)"),
    ("tagged", "the id ledger (phase P1)"),
    ("problems", "validation on open (phase P2)"),
];

/// The compatibility mode of a document whose settings name none: Word 2007's.
const DEFAULT_COMPATIBILITY_MODE: i64 = 12;

/// The core properties `describe` shows, in the order docx-agent reads them.
const PROPERTIES: [(&str, &str, &str); 3] = [
    ("title", DC, "title"),
    ("subject", DC, "subject"),
    ("author", DC, "creator"),
];
const DC: &str = "http://purl.org/dc/elements/1.1/";

/// `describe` of a Word document `name` at one version of its package. Cancellation is
/// checked once per part read.
///
/// # Errors
/// `timeout` when `cancel` asks to stop; `internal` when a part it reads is not XML or the
/// package has no main part.
pub fn describe(
    _args: &Describe,
    package: &Package,
    name: &str,
    cancel: &dyn Cancel,
) -> Result<Done<Value>, ToolError> {
    let reader = Reader { package, cancel };
    let main = main_part(package)
        .map_err(opc)?
        .ok_or_else(|| ToolError::internal("not a Word package: no main document part", "describe:main"))?;
    let mut data = Map::new();
    data.insert("name".into(), Value::from(name));
    data.insert(
        "compatibility_mode".into(),
        Value::from(reader.compatibility_mode(&main)?),
    );
    data.insert("properties".into(), Value::Object(reader.properties()?));
    data.insert("comments".into(), reader.comments(&main)?);
    let mut parts = vec![main.clone()];
    parts.extend(story_parts(package, &main).map_err(opc)?);
    let (notes, controls) = reader.notes_and_controls(&parts)?;
    data.insert("notes".into(), Value::Object(notes));
    data.insert("controls".into(), Value::from(controls));
    let mut done = Done::new(format!("Described {name}"));
    done.data = Some(Value::Object(data));
    Ok(done)
}

/// One version's parts, read with cancellation checked per part.
struct Reader<'a> {
    package: &'a Package,
    cancel: &'a dyn Cancel,
}

impl Reader<'_> {
    fn tree(&self, part: &str) -> Result<Option<Tree>, ToolError> {
        self.cancel.check().map_err(cancelled)?;
        self.package.tree(part).map_err(opc)
    }

    /// `w:compatSetting w:name="compatibilityMode"` in the settings' `w:compat`, or 12.
    fn compatibility_mode(&self, main: &str) -> Result<i64, ToolError> {
        let Some(part) = related(self.package, main, REL_SETTINGS).map_err(opc)? else {
            return Ok(DEFAULT_COMPATIBILITY_MODE);
        };
        let Some(tree) = self.tree(&part)? else {
            return Ok(DEFAULT_COMPATIBILITY_MODE);
        };
        let Some(compat) = tree.root().find(Some(W), "compat") else {
            return Ok(DEFAULT_COMPATIBILITY_MODE);
        };
        let setting = compat
            .child_elements()
            .filter(|e| e.name().is(Some(W), "compatSetting"))
            .find(|e| e.attribute(Some(W), "name") == Some("compatibilityMode"));
        Ok(setting
            .and_then(|e| python_int(e.attribute(Some(W), "val").unwrap_or("")))
            .unwrap_or(DEFAULT_COMPATIBILITY_MODE))
    }

    /// The title, subject and author the core properties set.
    fn properties(&self) -> Result<Map<String, Value>, ToolError> {
        let mut out = Map::new();
        let rels = crate::package::relationships(self.package, "").map_err(opc)?;
        let part = rels
            .into_iter()
            .filter(|r| r.rel_type == REL_CORE)
            .filter_map(|r| r.target_part)
            .find(|part| self.package.has_part(part));
        let Some(part) = part else {
            return Ok(out);
        };
        let Some(tree) = self.tree(&part)? else {
            return Ok(out);
        };
        for (key, namespace, local) in PROPERTIES {
            if let Some(element) = tree.root().find(Some(namespace), local) {
                out.insert(key.into(), Value::from(leading_text(element)));
            }
        }
        Ok(out)
    }

    /// Threads, open threads and comments by author, as `word_describe` counts them.
    fn comments(&self, main: &str) -> Result<Value, ToolError> {
        let records = self.comment_records(main)?;
        // A comment is found by its id: two records of one id read as the first.
        let first = |id: &str| records.iter().find(|r| r.id == id);
        let mut threads = 0_u64;
        let mut open = 0_u64;
        let mut by_author: Vec<(Option<&str>, u64)> = Vec::new();
        for record in &records {
            let Some(found) = first(&record.id) else { continue };
            if found.parent.is_none() {
                threads += 1;
                open += u64::from(!found.done);
            }
            let author = found.author.as_deref();
            match by_author.iter_mut().find(|(a, _)| *a == author) {
                Some((_, n)) => *n += 1,
                None => by_author.push((author, 1)),
            }
        }
        let mut counts = Map::new();
        for (author, n) in by_author {
            counts.insert(author.unwrap_or("null").to_owned(), Value::from(n));
        }
        let mut out = Map::new();
        out.insert("threads".into(), Value::from(threads));
        out.insert("open".into(), Value::from(open));
        out.insert("by_author".into(), Value::Object(counts));
        Ok(Value::Object(out))
    }

    fn comment_records(&self, main: &str) -> Result<Vec<CommentRecord>, ToolError> {
        let parts = related_parts(self.package, main, REL_COMMENTS).map_err(opc)?;
        let Some(part) = parts.into_iter().next().filter(|p| self.package.has_part(p)) else {
            return Ok(Vec::new());
        };
        let durable = self.by_para(main, REL_COMMENTS_IDS, W16CID, |node| {
            node.attribute(Some(W16CID), "durableId")
                .filter(|d| !d.is_empty())
                .map(str::to_owned)
        })?;
        let extended = self.by_para(main, REL_COMMENTS_EXTENDED, W15, |node| {
            let parent = node.attribute(Some(W15), "paraIdParent").unwrap_or("").to_uppercase();
            let done = matches!(node.attribute(Some(W15), "done").unwrap_or("0"), "1" | "true");
            Some(((!parent.is_empty()).then_some(parent), done))
        })?;
        let Some(tree) = self.tree(&part)? else {
            return Ok(Vec::new());
        };
        let mut records = Vec::new();
        let mut by_last: Vec<(String, String)> = Vec::new();
        for comment in tree.root().child_elements().filter(|e| e.name().is(Some(W), "comment")) {
            let last = comment_last_para(comment);
            let id = match lookup(&durable, &last) {
                Some(d) => format!("c:{d}"),
                None => format!("c#{}", comment.attribute(Some(W), "id").unwrap_or("None")),
            };
            let (parent_para, done) = lookup(&extended, &last).cloned().unwrap_or((None, false));
            if !last.is_empty() {
                match by_last.iter_mut().find(|(k, _)| *k == last) {
                    Some(slot) => slot.1.clone_from(&id),
                    None => by_last.push((last.clone(), id.clone())),
                }
            }
            records.push((
                CommentRecord {
                    id,
                    author: comment.attribute(Some(W), "author").map(str::to_owned),
                    done,
                    parent: None,
                },
                parent_para,
            ));
        }
        Ok(records
            .into_iter()
            .map(|(mut record, parent_para)| {
                record.parent = parent_para.and_then(|p| lookup(&by_last, &p).cloned());
                record
            })
            .collect())
    }

    /// The values `read` takes from each child of the first part `main` relates by
    /// `rel_type`, by its `paraId` (upper case) in `namespace`; the last of a `paraId` wins.
    fn by_para<T>(
        &self,
        main: &str,
        rel_type: &str,
        namespace: &str,
        read: impl Fn(&Element) -> Option<T>,
    ) -> Result<Vec<(String, T)>, ToolError> {
        let mut out: Vec<(String, T)> = Vec::new();
        let Some(part) = related(self.package, main, rel_type).map_err(opc)? else {
            return Ok(out);
        };
        let Some(tree) = self.tree(&part)? else {
            return Ok(out);
        };
        for node in tree.root().child_elements() {
            let para = node.attribute(Some(namespace), "paraId").unwrap_or("").to_uppercase();
            if let Some(value) = read(node) {
                match out.iter_mut().find(|(k, _)| *k == para) {
                    Some(slot) => slot.1 = value,
                    None => out.push((para, value)),
                }
            }
        }
        Ok(out)
    }

    /// Footnotes and endnotes by kind (separators left out), and the content controls of
    /// every story, as `word_describe` counts them.
    fn notes_and_controls(&self, parts: &[String]) -> Result<(Map<String, Value>, u64), ToolError> {
        let mut notes: Vec<(&str, u64)> = Vec::new();
        let mut controls = 0_u64;
        for part in parts {
            let Some(tree) = self.tree(part)? else { continue };
            let root = tree.root();
            controls += live_elements(root, W, "sdt").len() as u64;
            let (kind, child) = if root.name().is(Some(W), "footnotes") {
                ("footnote", "footnote")
            } else if root.name().is(Some(W), "endnotes") {
                ("endnote", "endnote")
            } else {
                continue;
            };
            let count = root
                .child_elements()
                .filter(|e| e.name().is(Some(W), child))
                .filter(|e| matches!(e.attribute(Some(W), "type"), None | Some("normal")))
                .count() as u64;
            if count == 0 {
                continue;
            }
            match notes.iter_mut().find(|(k, _)| *k == kind) {
                Some((_, n)) => *n += count,
                None => notes.push((kind, count)),
            }
        }
        let notes = notes.into_iter().map(|(k, n)| (k.to_owned(), Value::from(n))).collect();
        Ok((notes, controls))
    }
}

/// One comment as `describe` reads it.
struct CommentRecord {
    id: String,
    author: Option<String>,
    done: bool,
    parent: Option<String>,
}

/// The `w14:paraId` of a comment's last paragraph, upper case (`""` without one).
fn comment_last_para(comment: &Element) -> String {
    descendants(comment, W, "p")
        .last()
        .and_then(|p| p.attribute(Some(W14), "paraId").map(str::to_uppercase))
        .unwrap_or_default()
}

fn lookup<'a, T>(pairs: &'a [(String, T)], key: &str) -> Option<&'a T> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// A whole number as Python's `int(text)` reads the forms Word writes (spaces around, a
/// sign); `None` where it raises.
fn python_int(text: &str) -> Option<i64> {
    let text = text.trim();
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn opc(error: OpcError) -> ToolError {
    ToolError::internal(error.to_string(), "describe:package")
}

fn cancelled(error: BaseError) -> ToolError {
    match error {
        BaseError::Cancelled => ToolError::failure(
            Failure::Timeout,
            "describe stopped before it finished (the call was cancelled or ran past its deadline); nothing changed",
        ),
        other => ToolError::internal(other.to_string(), "describe:cancel"),
    }
}

#[cfg(test)]
mod tests {
    use super::python_int;

    #[test]
    fn whole_numbers_read_as_python_reads_them() {
        assert_eq!(python_int(" 15 "), Some(15));
        assert_eq!(python_int("+14"), Some(14));
        assert_eq!(python_int("-1"), Some(-1));
        assert_eq!(python_int(""), None);
        assert_eq!(python_int("15.0"), None);
        assert_eq!(python_int("x"), None);
    }
}

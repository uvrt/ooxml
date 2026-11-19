//! A placing tool's names for a place (docx-agent's `tools/positions.py`, round 40): the
//! names other placing tools give a place -- `where`, `target`, `after`, `before`,
//! `position`, `location`, `at`, `find` -- that this tool lacks, each read as the tool's
//! own field when, renamed, it says the same thing, and refused with the arguments that
//! say it this tool's way (rule 2) when the side is in the name.
//!
//! Owns: [`Places`] (a tool's place kind and the names it reads as its own) and
//! [`Places::meant`], the reading of one name given in a call.
//!
//! Must not: resolve addresses, or read anything but the call's arguments.

use crate::error::{Refusal, ToolError, TryCall, WayOut};
use crate::pyfmt::repr;
use serde_json::{Map, Value};

/// The names the placing tools give a place, in the order a tool's aliases list them.
pub const NAMES: [&str; 8] = [
    "where", "target", "after", "before", "position", "location", "at", "find",
];

/// Values whose prefix names the side: written as they are.
const SIDES: [&str; 4] = ["after:", "before:", "replace:", "end"];

/// What a tool's place is a place of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceKind {
    /// New blocks (`at`).
    Blocks,
    /// Text (`at`).
    Text,
    /// A note, field or control: a position, or a range's end (`at`).
    Mark,
    /// A picture: a position, or `after:`/`before:` a block (`at`).
    Picture,
    /// Beside a block: the `after` and `before` fields.
    Sides,
}

impl PlaceKind {
    /// The kind written `name` (`blocks`, `text`, `mark`, `picture`, `sides`).
    #[must_use]
    pub fn named(name: &str) -> Option<PlaceKind> {
        Some(match name {
            "blocks" => PlaceKind::Blocks,
            "text" => PlaceKind::Text,
            "mark" => PlaceKind::Mark,
            "picture" => PlaceKind::Picture,
            "sides" => PlaceKind::Sides,
            _ => return None,
        })
    }

    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            PlaceKind::Blocks => "blocks",
            PlaceKind::Text => "text",
            PlaceKind::Mark => "mark",
            PlaceKind::Picture => "picture",
            PlaceKind::Sides => "sides",
        }
    }

    /// The field an alias is read as: `after` beside a block, else `at`.
    #[must_use]
    pub fn field(self) -> &'static str {
        if self == PlaceKind::Sides {
            "after"
        } else {
            "at"
        }
    }
}

/// A placing tool's place: its kind and the names of [`NAMES`] it reads as its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    /// What it places.
    pub kind: PlaceKind,
    /// The names read as its own field, in [`NAMES`]' order.
    pub names: Vec<String>,
    /// The array field whose items are places too (`items`: `items[].after`), if any.
    pub items: Option<String>,
}

impl Places {
    /// The place of a tool of `kind` whose own names (parameters, or names that mean
    /// something else to it) are `own`; `find`: whether it takes `find` itself.
    #[must_use]
    pub fn new(kind: PlaceKind, own: &[&str], find: bool) -> Places {
        let names = NAMES
            .iter()
            .filter(|name| !own.contains(name) && !(find && **name == "find"))
            .map(|name| (*name).to_owned())
            .collect();
        Places {
            kind,
            names,
            items: None,
        }
    }

    /// These places, read in each item of the array field `field` too.
    #[must_use]
    pub fn in_items(mut self, field: &str) -> Places {
        self.items = Some(field.to_owned());
        self
    }

    /// The aliases these places export, each with the name it is written at: the names,
    /// then the items' (`items[].after`).
    #[must_use]
    pub fn alias_keys(&self) -> Vec<String> {
        let mut out = self.names.clone();
        if let Some(items) = &self.items {
            out.extend(self.names.iter().map(|name| format!("{items}[].{name}")));
        }
        out
    }

    /// What `name`, given in `node` (the call's arguments) to `tool`, is read as: the
    /// tool's own field, or `None` when it is no place (the validator then names it).
    ///
    /// # Errors
    /// `invalid_arguments` when renaming would change the meaning: the message says how to
    /// write it, the way out holds the call that works.
    pub fn meant(&self, tool: &str, name: &str, node: &Map<String, Value>) -> Result<Option<String>, ToolError> {
        let kind = self.kind;
        let field = kind.field();
        let Some(value) = node.get(name).and_then(Value::as_str).filter(|v| !v.is_empty()) else {
            return Ok(None);
        };
        let refuse = |message: String| refused(tool, kind, node, name, message, None, &[]);
        if name == "find" {
            let holding = repr(&Value::from(short(value)));
            return Err(refuse(format!(
                "{tool} places by {field}, an id, not by text: find_text gives the id of the paragraph \
                 holding {holding}; then {field}: 'after:<that id>'"
            )));
        }
        let (side, anchor, used) = read(name, value, node);
        let Some(anchor) = anchor else {
            return Ok(None);
        };
        if kind == PlaceKind::Sides {
            let Some(side) = side else {
                if anchor.starts_with("replace:") {
                    return Err(refuse(format!(
                        "{tool} places beside a block: give after or before, not replace"
                    )));
                }
                return Ok(Some(field.to_owned()));
            };
            return Err(refused(
                tool,
                kind,
                node,
                name,
                String::new(),
                Some((side, anchor)),
                &used,
            ));
        }
        let Some(written) = write(kind, side, &anchor) else {
            return Err(refuse(format!(
                "{tool} places by at: a position is p:<id>@<offset>; before a range is its start's offset"
            )));
        };
        if !used.contains(&"where") && written == value {
            return Ok(Some(field.to_owned()));
        }
        Err(refused(
            tool,
            kind,
            node,
            name,
            String::new(),
            Some((field, written)),
            &used,
        ))
    }
}

/// `(side, anchor, names used)` of a name given: `after: X` is (after, X); `where` reads
/// `target` (or a `ref` holding an address) as its anchor; no side for one the value says
/// itself, or none (after).
fn read<'a>(name: &'a str, value: &str, node: &Map<String, Value>) -> (Option<&'a str>, Option<String>, Vec<&'a str>) {
    let text = |key: &str| node.get(key).and_then(Value::as_str);
    match name {
        "after" | "before" => (Some(name), Some(value.to_owned()), vec![name]),
        "where" => {
            let side = match value {
                "after" => "after",
                "before" => "before",
                _ => return (None, None, vec!["where"]),
            };
            if let Some(target) = node.get("target").filter(|v| !v.is_null()) {
                let anchor = target.as_str().map(str::to_owned);
                return (Some(side), anchor, vec!["where", "target"]);
            }
            if let Some(reference) = text("ref").filter(|r| is_place(r)) {
                return (Some(side), Some(reference.to_owned()), vec!["where", "ref"]);
            }
            (Some(side), None, vec!["where"])
        }
        "target" => match node.get("where").filter(|v| !v.is_null()) {
            Some(place) => {
                let side = match place.as_str() {
                    Some("after") => Some("after"),
                    Some("before") => Some("before"),
                    _ => None,
                };
                (side, Some(value.to_owned()), vec!["target", "where"])
            }
            None => (None, Some(value.to_owned()), vec!["target"]),
        },
        _ => (None, Some(value.to_owned()), vec![name]),
    }
}

/// The tool's own `at` for `side` of `anchor`; `None` when it has no such place.
fn write(kind: PlaceKind, side: Option<&str>, anchor: &str) -> Option<String> {
    let Some(side) = side.filter(|_| !SIDES.iter().any(|s| anchor.starts_with(s))) else {
        return Some(anchor.to_owned());
    };
    match kind {
        PlaceKind::Blocks | PlaceKind::Text => Some(if side == "after" {
            anchor.to_owned()
        } else {
            format!("before:{anchor}")
        }),
        PlaceKind::Picture => Some(if anchor.contains('@') {
            anchor.to_owned()
        } else {
            format!("{side}:{anchor}")
        }),
        PlaceKind::Mark | PlaceKind::Sides => {
            if side == "after" {
                return Some(anchor.to_owned());
            }
            match anchor.split_once('@') {
                None if anchor.starts_with("p:") && !anchor.contains("..") => Some(format!("{anchor}@0")),
                None => None,
                Some((head, span)) => {
                    let start = span.split(':').next().unwrap_or("");
                    Some(format!("{head}@{}", start.split("..").next().unwrap_or("")))
                }
            }
        }
    }
}

/// Whether `value` reads as an address or a place, not as a ref name.
fn is_place(value: &str) -> bool {
    value.contains([':', '@', '$']) || value.contains("..")
}

/// `text`'s words joined by one space, cut to 40 characters with an ellipsis.
fn short(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= 40 {
        return text;
    }
    let mut cut: String = text.chars().take(39).collect();
    cut.push('…');
    cut
}

/// The refusal of a place named another tool's way: with `meant` (the field and value
/// that say it this tool's way) the message says how to write it and the way out is the
/// call rewritten without the names `drop`; else `message`, the options this tool takes.
fn refused(
    tool: &str,
    kind: PlaceKind,
    node: &Map<String, Value>,
    name: &str,
    message: String,
    meant: Option<(&str, String)>,
    drop: &[&str],
) -> ToolError {
    let Some((key, value)) = meant else {
        let options = if kind == PlaceKind::Sides {
            vec!["after", "before"]
        } else {
            vec!["at"]
        };
        let way_out = WayOut::names(options).unwrap_or_else(|| WayOut::one(Value::from("at")));
        return ToolError::refusal(Refusal::InvalidArguments, message, way_out).at(name);
    };
    let fields = format!("{key}: {}", repr(&Value::from(value.as_str())));
    let message = format!("{name} is another tool's way to name a place; {tool} takes {key}: write {fields}");
    let mut arguments: Map<String, Value> = node
        .iter()
        .filter(|(k, _)| !drop.contains(&k.as_str()) && k.as_str() != name)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    arguments.insert(key.to_owned(), Value::from(value));
    let call = TryCall {
        tool: tool.to_owned(),
        arguments,
        fill: Vec::new(),
    };
    ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(call)).at(name)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use serde_json::json;

    fn node(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn names_are_those_a_tool_lacks() {
        let places = Places::new(PlaceKind::Mark, &["at", "note"], true);
        assert_eq!(
            places.names,
            ["where", "target", "after", "before", "position", "location"]
        );
    }

    #[test]
    fn a_side_in_the_value_is_read_as_after() {
        let places = Places::new(PlaceKind::Sides, &["after", "before"], false);
        let call = node(json!({"at": "before:p:A"}));
        assert_eq!(
            places.meant("word_move", "at", &call).unwrap().as_deref(),
            Some("after")
        );
        let call = node(json!({"at": "replace:p:A"}));
        let error = places.meant("word_move", "at", &call).unwrap_err();
        assert_eq!(
            error.message(),
            "word_move places beside a block: give after or before, not replace"
        );
    }

    #[test]
    fn a_side_in_the_name_is_refused_with_the_call() {
        let places = Places::new(PlaceKind::Mark, &["at"], true);
        let call = node(json!({"doc": "d1", "before": "p:A@3:5"}));
        let error = places.meant("word_notes", "before", &call).unwrap_err();
        assert_eq!(
            error.message(),
            "before is another tool's way to name a place; word_notes takes at: write at: 'p:A@3'"
        );
        let way_out = error.way_out().and_then(WayOut::try_call).unwrap();
        assert_eq!(
            Value::Object(way_out.arguments.clone()),
            json!({"doc": "d1", "at": "p:A@3"})
        );
        let call = node(json!({"after": "p:A@3:5"}));
        assert_eq!(
            places.meant("word_notes", "after", &call).unwrap().as_deref(),
            Some("at")
        );
    }

    #[test]
    fn find_is_refused_for_a_tool_placing_by_id() {
        let places = Places::new(PlaceKind::Blocks, &["at"], false);
        let call = node(json!({"find": "  some   text "}));
        let error = places.meant("word_copy_from", "find", &call).unwrap_err();
        assert_eq!(
            error.message(),
            "word_copy_from places by at, an id, not by text: find_text gives the id of the paragraph \
             holding 'some text'; then at: 'after:<that id>'"
        );
    }
}

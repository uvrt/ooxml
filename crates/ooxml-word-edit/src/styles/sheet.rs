//! Owns: the style sheet as an edit reads and writes it ([`Sheet`]): the styles part's
//! editable copy (made in the edit's [`Space`]), its `w:style` children read as
//! docx-agent's `Style` (id, name, kind, based on, default, aliases), found by name as
//! `Styles.find` finds them, a new style's id (`_new_id`), the references other styles
//! make to one (`_style_references`), and the theme colours and child orders writing
//! needs.
//!
//! Must not: write but into the copy, or decide what an edit means.

use crate::doc::Key;
use crate::format::ThemeColours;
use crate::names::W;
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::casefold;
use std::sync::Arc;

/// The relationship type of the styles part.
pub(crate) const REL_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";

/// The `w:` references a style makes to another (`_style_references`' tags).
pub(crate) const STYLE_REFERENCES: &[&str] = &[
    "basedOn",
    "next",
    "link",
    "pStyle",
    "rStyle",
    "tblStyle",
    "numStyleLink",
    "styleLink",
];

/// One `w:style` as docx-agent's `Style` reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// The node in the copy.
    pub(crate) node: Id,
    /// `w:styleId` (`""` where none).
    pub(crate) id: String,
    /// `w:name`.
    pub(crate) name: Option<String>,
    /// `w:type` (`paragraph` where none).
    pub(crate) kind: String,
    /// `w:basedOn`.
    pub(crate) based_on: Option<String>,
    /// `w:default` on.
    pub(crate) default: bool,
    /// `w:aliases`.
    pub(crate) aliases: Vec<String>,
}

/// An `ST_OnOff` as docx-agent's style code reads it (`1`, `true`, `on`).
pub(crate) fn on(value: Option<&str>) -> bool {
    matches!(value.map(str::to_lowercase).as_deref(), Some("1" | "true" | "on"))
}

/// The style sheet of one edit.
pub(crate) struct Sheet {
    /// The styles part.
    pub(crate) part: String,
    /// Its root in the copy.
    pub(crate) root: Id,
    /// The theme's colours, for a colour named by its theme name.
    pub(crate) colours: ThemeColours,
    /// The schema's child orders.
    pub(crate) orders: Arc<crate::doc::Shared>,
}

impl Sheet {
    /// The style sheet of `sink`'s document, read (its key declared) and copied into
    /// `space`; `None` when the document has no styles part.
    ///
    /// # Errors
    /// When a part is not XML.
    pub(crate) fn open(sink: &mut EditSink<'_, '_>, space: &mut Space) -> Result<Option<Sheet>> {
        let docx = sink.docx()?;
        let Some(part) = ooxml_word_model::parts::related(sink.doc().package(), &docx.main, REL_STYLES)? else {
            return Ok(None);
        };
        sink.read(Key::Part(Arc::from(part.as_str())));
        let colours = ThemeColours::read(sink.doc(), &docx)?;
        let held = sink.doc().clone();
        let Some(work) = space.work(&held, &part)? else {
            return Ok(None);
        };
        let root = work.root();
        Ok(Some(Sheet {
            part,
            root,
            colours,
            orders: held.shared_handle(),
        }))
    }

    /// The child orders.
    pub(crate) fn orders(&self) -> &Orders {
        self.orders.facts.orders()
    }

    /// The copy.
    ///
    /// # Errors
    /// When the copy is gone (never: it is made on open).
    pub(crate) fn work<'s>(&self, space: &'s mut Space) -> Result<&'s mut Work> {
        space
            .get_mut(&self.part)
            .ok_or_else(|| EditError::NoPart(self.part.clone()))
    }

    /// The copy, read only.
    ///
    /// # Errors
    /// As [`Sheet::work`].
    pub(crate) fn view<'s>(&self, space: &'s Space) -> Result<&'s Work> {
        space
            .get(&self.part)
            .ok_or_else(|| EditError::NoPart(self.part.clone()))
    }
}

/// `w:val` of `node`'s first `w:{local}` child.
pub(crate) fn child_val<'w>(work: &'w Work, node: Id, local: &str) -> Option<&'w str> {
    work.find(node, W, local).and_then(|c| work.get(c, Some(W), "val"))
}

/// The style read from its node.
pub(crate) fn entry(work: &Work, node: Id) -> Entry {
    Entry {
        node,
        id: work.get(node, Some(W), "styleId").unwrap_or("").to_owned(),
        name: child_val(work, node, "name").map(str::to_owned),
        kind: work
            .get(node, Some(W), "type")
            .filter(|t| !t.is_empty())
            .unwrap_or("paragraph")
            .to_owned(),
        based_on: child_val(work, node, "basedOn").map(str::to_owned),
        default: on(Some(work.get(node, Some(W), "default").unwrap_or("0"))),
        aliases: child_val(work, node, "aliases")
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_owned)
            .collect(),
    }
}

/// Every `w:style` of the sheet, in order.
pub(crate) fn entries(work: &Work, root: Id) -> Vec<Entry> {
    work.children(root)
        .iter()
        .filter(|c| work.is(**c, W, "style"))
        .map(|c| entry(work, *c))
        .collect()
}

/// The style `name` names among those of `kind` (any kind for `None`), as docx-agent's
/// `Styles.find`: by name in any case, then an alias, then its id, then its id in any case.
pub(crate) fn find(work: &Work, root: Id, name: &str, kind: Option<&str>) -> Option<Entry> {
    let styles: Vec<Entry> = entries(work, root)
        .into_iter()
        .filter(|s| kind.is_none_or(|k| s.kind == k))
        .collect();
    let wanted = casefold(name);
    let tests: [&dyn Fn(&Entry) -> bool; 4] = [
        &|s| casefold(s.name.as_deref().unwrap_or("")) == wanted,
        &|s| s.aliases.iter().any(|a| casefold(a) == wanted),
        &|s| s.id == name,
        &|s| casefold(&s.id) == wanted,
    ];
    tests.iter().find_map(|test| styles.iter().find(|s| test(s)).cloned())
}

/// [`find`] on a version's style sheet: the `w:style` `name` names.
pub(crate) fn find_in(
    docx: &ooxml_word_model::docx::Docx,
    name: &str,
    kind: Option<&str>,
) -> Option<Arc<ooxml_xml::Element>> {
    use ooxml_word_model::docx::{style_id, style_kind, style_name};
    let styles: Vec<&Arc<ooxml_xml::Element>> = docx
        .styles()
        .iter()
        .filter(|s| kind.is_none_or(|k| style_kind(s) == k))
        .collect();
    let wanted = casefold(name);
    let aliases = |s: &ooxml_xml::Element| {
        s.find(Some(W), "aliases")
            .and_then(|a| a.attribute(Some(W), "val"))
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .any(|a| !a.is_empty() && casefold(a) == wanted)
    };
    let tests: [&dyn Fn(&ooxml_xml::Element) -> bool; 4] = [
        &|s| casefold(style_name(s).unwrap_or("")) == wanted,
        &aliases,
        &|s| style_id(s) == name,
        &|s| casefold(style_id(s)) == wanted,
    ];
    tests
        .iter()
        .find_map(|test| styles.iter().find(|s| test(s)).map(|s| Arc::clone(s)))
}

/// The style with id `id`.
pub(crate) fn by_id(work: &Work, root: Id, id: &str) -> Option<Id> {
    work.children(root)
        .iter()
        .copied()
        .find(|c| work.is(*c, W, "style") && work.get(*c, Some(W), "styleId") == Some(id))
}

/// A style's name quoted for a message, its id where it has none (`_style_of`).
pub(crate) fn quoted_name(work: &Work, root: Id, id: &str) -> String {
    let name = by_id(work, root, id).and_then(|n| child_val(work, n, "name"));
    ooxml_base::pyfmt::repr_str(name.unwrap_or(id))
}

/// A new style's id: its name without spaces or punctuation, numbered when taken
/// (`_new_id`).
pub(crate) fn new_id(work: &Work, root: Id, name: &str) -> String {
    let base: String = name.chars().filter(char::is_ascii_alphanumeric).collect();
    let base = if base.is_empty() { "Style".to_owned() } else { base };
    let taken: Vec<String> = entries(work, root).iter().map(|s| casefold(&s.id)).collect();
    let mut candidate = base.clone();
    let mut number = 1;
    while taken.contains(&casefold(&candidate)) {
        number += 1;
        candidate = format!("{base}{number}");
    }
    candidate
}

/// The references other styles make to `id` (`_style_references`): each element.
pub(crate) fn style_references(work: &Work, root: Id, id: &str) -> Vec<Id> {
    let mut out = Vec::new();
    for style in work.children(root).iter().copied().filter(|c| work.is(*c, W, "style")) {
        if work.get(style, Some(W), "styleId") == Some(id) {
            continue;
        }
        out.extend(
            work.iter(style, W, STYLE_REFERENCES)
                .into_iter()
                .filter(|r| work.get(*r, Some(W), "val") == Some(id)),
        );
    }
    out
}

/// A new `w:{local}` element with `w:` attributes, detached.
pub(crate) fn make(work: &mut Work, local: &str, attributes: &[(&str, &str)]) -> Id {
    crate::review::xml::make(work, local, attributes)
}

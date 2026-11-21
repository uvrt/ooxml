//! Owns: a saved file with what Word keeps but never draws taken out, as docx-agent's
//! `edit/cleaning.py` takes it (round 16, F11; measured in Word 16.106: the cleaned file
//! opens without a prompt and draws as the original): parts nothing uses
//! ([`super::orphans`]); settings' external `attachedTemplate` and its relationship; every
//! `w:rsid*` attribute of the WordprocessingML parts and settings' `w:rsids` list; and
//! `webSettings`' `w:divs` with every `w:divId` -- unless a division sets a margin or a
//! border, which Word draws: then the divisions stay whole. [`Cleaned`] says what was
//! taken, as the tool reports it.
//!
//! Must not: touch the document in the session (only the bytes given are cleaned).

use super::orphans::{drop_orphans, size};
use crate::doc::ModelError;
use crate::names::W;
use crate::new_part::edit_part;
use crate::work::{Id, Work};
use crate::Result;
use ooxml_base::Limits;
use ooxml_opc::name::rels_for;
use ooxml_opc::Package;
use ooxml_word_model::parts::{main_part, related, relationships, REL_SETTINGS};
use serde_json::{json, Map, Value};
use std::sync::Arc;

const REL_ATTACHED_TEMPLATE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/attachedTemplate";
const REL_WEB_SETTINGS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/webSettings";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
/// A division's margins, drawn when not 0 (ECMA-376 17.15.2.8 `div`).
const DRAWN: [&str; 4] = ["marLeft", "marRight", "marTop", "marBottom"];

/// What [`clean`] took out: each kind only when something of it was there, its `bytes` the
/// XML (or part) bytes it held, uncompressed; the files' sizes before and after.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cleaned {
    /// The template link's target and bytes.
    pub attached_template: Option<(String, i64)>,
    /// The rsid attributes, the ids settings listed, and their bytes.
    pub rsids: Option<(u64, u64, i64)>,
    /// The divisions and their bytes.
    pub web_divs: Option<(u64, i64)>,
    /// The parts taken out, `(part, bytes)`, in name order.
    pub orphaned_parts: Vec<(String, u64)>,
    /// The file's size before.
    pub before: usize,
    /// The file's size after.
    pub after: usize,
}

impl Cleaned {
    /// As the tool reports it (docx-agent's `cleaned`): `{"orphaned_parts": {"count",
    /// "bytes", "parts"}, "rsids": {"attributes", "listed", "bytes"}, "web_divs": {"count",
    /// "bytes"}, "attached_template": {"target", "bytes"}, "bytes": {"before", "after"}}`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        if let Some((target, bytes)) = &self.attached_template {
            out.insert("attached_template".into(), json!({"target": target, "bytes": bytes}));
        }
        if let Some((attributes, listed, bytes)) = self.rsids {
            out.insert(
                "rsids".into(),
                json!({"attributes": attributes, "listed": listed, "bytes": bytes}),
            );
        }
        if let Some((count, bytes)) = self.web_divs {
            out.insert("web_divs".into(), json!({"count": count, "bytes": bytes}));
        }
        if !self.orphaned_parts.is_empty() {
            let parts: Vec<&str> = self.orphaned_parts.iter().take(20).map(|(p, _)| p.as_str()).collect();
            out.insert(
                "orphaned_parts".into(),
                json!({"count": self.orphaned_parts.len(),
                       "bytes": self.orphaned_parts.iter().map(|(_, b)| b).sum::<u64>(),
                       "parts": parts}),
            );
        }
        out.insert("bytes".into(), json!({"before": self.before, "after": self.after}));
        Value::Object(out)
    }
}

/// `data` (a .docx, .dotx, .docm or .dotm) with the weight above taken out (the same bytes
/// where there was none), and what was taken.
///
/// # Errors
/// When `data` is not a package, or a part it reads is not XML.
pub fn clean(data: &[u8], limits: &Limits) -> Result<(Vec<u8>, Cleaned)> {
    let mut package = Package::open(Arc::from(data), limits).map_err(ModelError::from)?;
    let mut report = Cleaned::default();
    let main = main_part(&package).map_err(ModelError::from)?;
    let settings = match &main {
        Some(main) => related(&package, main, REL_SETTINGS).map_err(ModelError::from)?,
        None => None,
    };
    if let Some(settings) = &settings {
        report.attached_template = attached_template(&mut package, settings)?;
    }
    let rsids = rsids(&mut package, settings.as_deref())?;
    if rsids.0 > 0 || rsids.1 > 0 {
        report.rsids = Some(rsids);
    }
    if let Some(main) = &main {
        report.web_divs = web_divs(&mut package, main)?;
    }
    let (dropped, removed) = drop_orphans(&package)?;
    package = dropped;
    report.orphaned_parts = removed;
    let something = report.attached_template.is_some()
        || report.rsids.is_some()
        || report.web_divs.is_some()
        || !report.orphaned_parts.is_empty();
    let out = if something {
        package.save().map_err(ModelError::from)?
    } else {
        data.to_vec()
    };
    report.before = data.len();
    report.after = out.len();
    Ok((out, report))
}

/// The part's size as a signed count, for differences.
fn signed(package: &Package, part: &str) -> Result<i64> {
    Ok(i64::try_from(size(package, part)?).unwrap_or(i64::MAX))
}

/// Settings' external `attachedTemplate` and its relationship taken out: the link's target
/// and the bytes settings lost.
fn attached_template(package: &mut Package, settings: &str) -> Result<Option<(String, i64)>> {
    let Some(rel) = relationships(package, settings)
        .map_err(ModelError::from)?
        .into_iter()
        .find(|r| r.rel_type == REL_ATTACHED_TEMPLATE && r.external)
    else {
        return Ok(None);
    };
    let before = signed(package, settings)?;
    let id = rel.id.clone();
    *package = edit_part(package, settings, |work| {
        let found = work
            .descendants(work.root())
            .into_iter()
            .find(|n| work.is(*n, W, "attachedTemplate") && work.get(*n, Some(R), "id") == Some(id.as_str()));
        if let Some(node) = found {
            plain_remove(work, node);
        }
    })?;
    let rels = rels_for(settings);
    *package = edit_part(package, &rels, |work| {
        let found = work
            .children(work.root())
            .iter()
            .copied()
            .find(|n| work.is(*n, RELS_NS, "Relationship") && work.get(*n, None, "Id") == Some(id.as_str()));
        if let Some(node) = found {
            crate::content_types::remove_keeping_layout(work, node);
        }
    })?;
    Ok(Some((rel.target, before - signed(package, settings)?)))
}

/// lxml's `parent.remove(node)`: the node's tail goes with it.
fn plain_remove(work: &mut Work, node: Id) {
    work.set_tail(node, "");
    work.remove(node);
}

/// The WordprocessingML parts: `.xml` parts whose content type names WordprocessingML.
fn word_parts(package: &Package) -> Result<Vec<String>> {
    let types = package.content_types().map_err(ModelError::from)?;
    Ok(package
        .part_names()
        .into_iter()
        .filter(|p| {
            p.to_ascii_lowercase().ends_with(".xml") && types.of(p).is_some_and(|t| t.contains("wordprocessingml"))
        })
        .collect())
}

/// Every `w:rsid*` attribute and settings' `w:rsids` taken out: how many attributes, how
/// many ids the list held, the bytes.
fn rsids(package: &mut Package, settings: Option<&str>) -> Result<(u64, u64, i64)> {
    let (mut attributes, mut listed, mut saved) = (0u64, 0u64, 0i64);
    for part in word_parts(package)? {
        let data = package
            .read(&part)
            .map_err(ModelError::from)?
            .unwrap_or_else(|| Arc::from(&[][..]));
        if !data.windows(4).any(|w| w == b"rsid") {
            continue;
        }
        let mut count = 0u64;
        let mut list = 0u64;
        let is_settings = settings == Some(part.as_str());
        let edited = edit_part(package, &part, |work| {
            for node in work.descendants(work.root()) {
                let names: Vec<String> = work
                    .attributes(node)
                    .iter()
                    .filter(|a| a.name.namespace() == Some(W) && a.name.local().starts_with("rsid"))
                    .map(|a| a.name.local().to_owned())
                    .collect();
                count += names.len() as u64;
                for name in names {
                    work.remove_attr(node, Some(W), &name);
                }
            }
            if is_settings {
                let lists: Vec<Id> = work
                    .descendants(work.root())
                    .into_iter()
                    .filter(|n| work.is(*n, W, "rsids"))
                    .collect();
                for node in lists {
                    list += work.children(node).iter().filter(|c| work.is_element(**c)).count() as u64;
                    plain_remove(work, node);
                }
            }
        })?;
        let found_list = is_settings && list > 0;
        if count > 0 || found_list {
            *package = edited;
            saved += data.len() as i64 - signed(package, &part)?;
        }
        attributes += count;
        listed += list;
    }
    Ok((attributes, listed, saved))
}

/// Whether a division sets something Word draws: a border side, or a margin not 0.
fn drawn(division: &ooxml_xml::Element) -> bool {
    let child = |local: &str| division.child_elements().find(|e| e.name().is(Some(W), local));
    if let Some(border) = child("divBdr") {
        if border
            .child_elements()
            .any(|side| !matches!(side.attribute(Some(W), "val"), None | Some("nil" | "none")))
        {
            return true;
        }
    }
    DRAWN.iter().any(|name| {
        child(name).is_some_and(|node| !matches!(node.attribute(Some(W), "val").unwrap_or("0"), "0" | "auto"))
    })
}

/// `webSettings`' `w:divs` and every `w:divId` taken out -- unless a division is drawn:
/// how many divisions, the bytes.
fn web_divs(package: &mut Package, main: &str) -> Result<Option<(u64, i64)>> {
    let Some(web) = related(package, main, REL_WEB_SETTINGS).map_err(ModelError::from)? else {
        return Ok(None);
    };
    let Some(tree) = package.tree(&web).map_err(ModelError::from)? else {
        return Ok(None);
    };
    let root = tree.root();
    if !root.child_elements().any(|e| e.name().is(Some(W), "divs")) {
        return Ok(None);
    }
    let divisions: Vec<&ooxml_xml::Element> = descendants(root).filter(|e| e.name().is(Some(W), "div")).collect();
    if divisions.iter().any(|d| drawn(d)) {
        return Ok(None);
    }
    let before = signed(package, &web)?;
    let count = divisions.len() as u64;
    *package = edit_part(package, &web, |work| {
        let root = work.root();
        let tops: Vec<Id> = work
            .children(root)
            .iter()
            .copied()
            .filter(|c| work.is(*c, W, "divs"))
            .collect();
        tops.into_iter().for_each(|n| plain_remove(work, n));
    })?;
    let mut saved = before - signed(package, &web)?;
    for part in word_parts(package)? {
        let data = package
            .read(&part)
            .map_err(ModelError::from)?
            .unwrap_or_else(|| Arc::from(&[][..]));
        if !data.windows(5).any(|w| w == b"divId") {
            continue;
        }
        let mut found = false;
        let edited = edit_part(package, &part, |work| {
            let ids: Vec<Id> = work
                .descendants(work.root())
                .into_iter()
                .filter(|n| work.is(*n, W, "divId"))
                .collect();
            found = !ids.is_empty();
            ids.into_iter().for_each(|n| plain_remove(work, n));
        })?;
        if found {
            *package = edited;
            saved += data.len() as i64 - signed(package, &part)?;
        }
    }
    Ok(Some((count, saved)))
}

/// `root` and every element below it, in document order.
fn descendants(root: &ooxml_xml::Element) -> impl Iterator<Item = &ooxml_xml::Element> {
    let mut stack = vec![root];
    std::iter::from_fn(move || {
        let next = stack.pop()?;
        let children: Vec<&ooxml_xml::Element> = next.child_elements().map(|e| &**e).collect();
        stack.extend(children.into_iter().rev());
        Some(next)
    })
}

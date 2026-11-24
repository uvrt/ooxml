//! Owns: a paragraph style linked to a list level and unlinked, as Word's "Link level to
//! style" does it (docx-agent's `edit/numbering.py`: `link_style_level`, `unlink_style`,
//! `inheriting_styles`, `style_list`, `definition_of`, `defined_levels`; measured, rounds
//! 29 and 30), on the edit's copies of the style sheet and the numbering part -- read as
//! they stand mid-edit, as docx-agent reads its tree.
//!
//! Must not: write but into the copies, or decide what the tool says.

use super::sheet::{child_val, make, Sheet};
use crate::names::W;
use crate::review::space::Space;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::xmlread::py_int;

/// The levels a list has.
pub(crate) const LEVELS: i64 = 9;

/// The numbering part's copy: its part and root.
#[derive(Debug, Clone)]
pub(crate) struct Numbers {
    pub(crate) part: String,
    pub(crate) root: Id,
}

fn int(work: &Work, node: Id, local: &str) -> Option<i64> {
    work.get(node, Some(W), local).and_then(py_int)
}

fn children(work: &Work, node: Id, local: &str) -> Vec<Id> {
    work.children(node)
        .iter()
        .copied()
        .filter(|c| work.is(*c, W, local))
        .collect()
}

/// `w:num` by `numId`, in the part's order (the last of one number wins its value).
fn nums(work: &Work, root: Id) -> Vec<(i64, Id)> {
    listed(work, root, "num", "numId")
}

fn abstracts(work: &Work, root: Id) -> Vec<(i64, Id)> {
    listed(work, root, "abstractNum", "abstractNumId")
}

fn listed(work: &Work, root: Id, local: &str, attribute: &str) -> Vec<(i64, Id)> {
    let mut out: Vec<(i64, Id)> = Vec::new();
    for node in children(work, root, local) {
        if let Some(n) = int(work, node, attribute) {
            match out.iter_mut().find(|(k, _)| *k == n) {
                Some(slot) => slot.1 = node,
                None => out.push((n, node)),
            }
        }
    }
    out
}

fn get(list: &[(i64, Id)], key: Option<i64>) -> Option<Id> {
    let key = key?;
    list.iter().find(|(k, _)| *k == key).map(|(_, n)| *n)
}

fn abstract_of(work: &Work, root: Id, num_id: i64) -> Option<i64> {
    let num = get(&nums(work, root), Some(num_id))?;
    work.find(num, W, "abstractNumId").and_then(|r| int(work, r, "val"))
}

fn nsid(work: &Work, node: Id) -> Option<String> {
    work.find(node, W, "nsid")
        .map(|n| work.get(n, Some(W), "val").unwrap_or("").to_uppercase())
}

/// The first definition with `abstract_id`'s `w:nsid` (`Numbering.canonical`).
fn canonical(work: &Work, root: Id, abstract_id: Option<i64>) -> Option<i64> {
    let all = abstracts(work, root);
    let value = get(&all, abstract_id).and_then(|n| nsid(work, n)).unwrap_or_default();
    if value.is_empty() {
        return abstract_id;
    }
    all.iter()
        .find(|(_, n)| nsid(work, *n).as_deref() == Some(value.as_str()))
        .map_or(abstract_id, |(k, _)| Some(*k))
}

/// The `w:numId` a style's own `w:numPr` names (`_style_num`): the first style of that id.
fn style_num(styles: &Work, sroot: Id, style_id: &str) -> Option<i64> {
    let node = super::sheet::by_id(styles, sroot, style_id)?;
    own_num_id(styles, node).and_then(|n| int(styles, n, "val"))
}

fn own_num_id(work: &Work, style: Id) -> Option<Id> {
    let properties = work.find(style, W, "pPr")?;
    let numbering = work.find(properties, W, "numPr")?;
    work.find(numbering, W, "numId")
}

/// The definition `num_id` numbers with (`definition_of`): through a `w:numStyleLink` to
/// the list style's own.
fn definition_of(styles: &Work, sroot: Id, numbers: &Work, nroot: Id, num_id: i64) -> Option<Id> {
    let mut seen = Vec::new();
    let mut current = num_id;
    while !seen.contains(&current) {
        seen.push(current);
        let all = abstracts(numbers, nroot);
        let found = get(&all, canonical(numbers, nroot, abstract_of(numbers, nroot, current)))?;
        let link = numbers
            .find(found, W, "numStyleLink")
            .and_then(|l| numbers.get(l, Some(W), "val"));
        match link.and_then(|l| style_num(styles, sroot, l)).filter(|n| *n != 0) {
            None => return Some(found),
            Some(next) => current = next,
        }
    }
    None
}

/// `(ilvl, w:lvl)` of a definition and of every level override of an instance over it.
fn definition_levels(styles: &Work, sroot: Id, numbers: &Work, nroot: Id, definition: Id) -> Vec<(Option<i64>, Id)> {
    let mut out: Vec<(Option<i64>, Id)> = children(numbers, definition, "lvl")
        .into_iter()
        .map(|l| (int(numbers, l, "ilvl"), l))
        .collect();
    for (num_id, num) in nums(numbers, nroot) {
        if definition_of(styles, sroot, numbers, nroot, num_id) != Some(definition) {
            continue;
        }
        for over in children(numbers, num, "lvlOverride") {
            if let Some(lvl) = numbers.find(over, W, "lvl") {
                out.push((int(numbers, over, "ilvl"), lvl));
            }
        }
    }
    out
}

/// The levels the definition `num_id` numbers with defines (`defined_levels`).
pub(crate) fn defined_levels(space: &Space, sheet: &Sheet, numbers: Option<&Numbers>, num_id: i64) -> Vec<i64> {
    let (Some(styles), Some(n)) = (space.get(&sheet.part), numbers) else {
        return Vec::new();
    };
    let Some(work) = space.get(&n.part) else {
        return Vec::new();
    };
    let Some(definition) = definition_of(styles, sheet.root, work, n.root, num_id) else {
        return Vec::new();
    };
    let mut out: Vec<i64> = children(work, definition, "lvl")
        .into_iter()
        .filter_map(|l| int(work, l, "ilvl"))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The instances, by number.
pub(crate) fn list_numbers(space: &Space, numbers: Option<&Numbers>) -> Vec<i64> {
    numbers
        .and_then(|n| space.get(&n.part).map(|w| nums(w, n.root)))
        .map(|all| all.into_iter().map(|(k, _)| k).collect())
        .unwrap_or_default()
}

fn last_by_id(work: &Work, root: Id, id: &str) -> Option<Id> {
    work.children(root)
        .iter()
        .rev()
        .copied()
        .find(|c| work.is(*c, W, "style") && work.get(*c, Some(W), "styleId") == Some(id))
}

/// `(numId, ilvl)` style `id` numbers with as Word numbers it (`style_list`), read from the
/// copies as they stand.
pub(crate) fn style_list(space: &Space, sheet: &Sheet, numbers: Option<&Numbers>, id: &str) -> Option<(i64, i64)> {
    let styles = space.get(&sheet.part)?;
    let mut current = Some(id.to_owned());
    let mut seen: Vec<String> = Vec::new();
    let (mut num_id, mut level, mut holder) = (None, None, None);
    while let Some(at) = current.take().filter(|c| !c.is_empty() && !seen.contains(c)) {
        let Some(node) = last_by_id(styles, sheet.root, &at) else {
            break;
        };
        let numbering = styles.find(node, W, "pPr").and_then(|p| styles.find(p, W, "numPr"));
        if let Some(numbering) = numbering {
            if num_id.is_none() {
                if let Some(r) = styles.find(numbering, W, "numId") {
                    num_id = int(styles, r, "val");
                    holder = Some(at.clone());
                }
            }
            if level.is_none() {
                if let Some(r) = styles.find(numbering, W, "ilvl") {
                    level = int(styles, r, "val");
                }
            }
        }
        current = child_val(styles, node, "basedOn").map(str::to_owned);
        seen.push(at);
    }
    let level = level.unwrap_or(0);
    let n = numbers?;
    let work = space.get(&n.part)?;
    let num_id = num_id.filter(|v| *v != 0)?;
    get(&nums(work, n.root), Some(num_id))?;
    get(&abstracts(work, n.root), abstract_of(work, n.root, num_id))?;
    let lvl = level_of(styles, sheet.root, work, n.root, num_id, level, 0);
    let tied = lvl
        .and_then(|l| work.find(l, W, "pStyle"))
        .and_then(|t| work.get(t, Some(W), "val"))
        .filter(|v| !v.is_empty());
    if tied.is_some_and(|t| Some(t) != holder.as_deref()) {
        return None;
    }
    Some((num_id, level))
}

/// The `w:lvl` that applies to `level` of `num_id` (`Numbering.level`).
fn level_of(styles: &Work, sroot: Id, work: &Work, root: Id, num_id: i64, level: i64, depth: usize) -> Option<Id> {
    let num = get(&nums(work, root), Some(num_id))?;
    for over in children(work, num, "lvlOverride") {
        if int(work, over, "ilvl") == Some(level) {
            if let Some(lvl) = work.find(over, W, "lvl") {
                return Some(lvl);
            }
        }
    }
    let all = abstracts(work, root);
    let definition = get(&all, canonical(work, root, abstract_of(work, root, num_id)))?;
    if let Some(link) = work.find(definition, W, "numStyleLink") {
        let linked = work.get(link, Some(W), "val").and_then(|l| style_num(styles, sroot, l));
        if let Some(linked) = linked.filter(|l| *l != num_id && depth < 16) {
            return level_of(styles, sroot, work, root, linked, level, depth + 1);
        }
    }
    children(work, definition, "lvl")
        .into_iter()
        .find(|l| int(work, *l, "ilvl") == Some(level))
}

/// A style's own `w:numPr` taken out of its `w:pPr`, and its `w:ind` with `indent`
/// (`_drop_style_numbering`); whether anything changed.
fn drop_numbering(work: &mut Work, node: Id, indent: bool) -> bool {
    let Some(properties) = work.find(node, W, "pPr") else {
        return false;
    };
    let mut changed = false;
    let locals: &[&str] = if indent { &["numPr", "ind"] } else { &["numPr"] };
    for local in locals {
        if let Some(found) = work.find(properties, W, local) {
            work.remove(found);
            changed = true;
        }
    }
    if work.children(properties).is_empty() {
        work.remove(properties);
    }
    changed
}

fn numbers_work<'s>(space: &'s mut Space, numbers: &Numbers) -> Result<&'s mut Work> {
    space
        .get_mut(&numbers.part)
        .ok_or_else(|| EditError::no_part(numbers.part.clone()))
}

/// Link style `id` to `level` of `num_id` (`link_style_level`); the ids of the styles
/// that held the level before, unlinked.
///
/// # Errors
/// When the list has no such level.
pub(crate) fn link(
    space: &mut Space,
    sheet: &Sheet,
    numbers: &Numbers,
    id: &str,
    (num_id, level): (i64, i64),
) -> Result<Vec<String>> {
    let no_level = || EditError::Refused(format!("list {num_id} has no level {level}"));
    let (definition, levels) = {
        let styles = sheet.view(space)?;
        let work = space.get(&numbers.part).ok_or_else(no_level)?;
        let definition = definition_of(styles, sheet.root, work, numbers.root, num_id).ok_or_else(no_level)?;
        (
            definition,
            definition_levels(styles, sheet.root, work, numbers.root, definition),
        )
    };
    if !defined_levels(space, sheet, Some(numbers), num_id).contains(&level) {
        return Err(no_level());
    }
    let orders = sheet.orders();
    let mut unlinked: Vec<String> = Vec::new();
    {
        let work = numbers_work(space, numbers)?;
        for (ilvl, lvl) in levels {
            let tied = work.find(lvl, W, "pStyle");
            let holder = tied.and_then(|t| work.get(t, Some(W), "val")).map(str::to_owned);
            if ilvl == Some(level) && holder.as_deref() != Some(id) {
                if let Some(h) = holder.filter(|h| !h.is_empty() && !unlinked.contains(h)) {
                    unlinked.push(h);
                }
                let tied = tied.unwrap_or_else(|| {
                    let made = make(work, "pStyle", &[]);
                    crate::marks::insert_in_order(work, lvl, made, orders);
                    made
                });
                crate::review::xml::set_attr(work, tied, "val", id);
            } else if ilvl != Some(level) && holder.as_deref() == Some(id) {
                if let Some(tied) = tied {
                    work.remove(tied);
                }
            }
        }
    }
    for holder in &unlinked {
        let (node, theirs_ok) = {
            let styles = sheet.view(space)?;
            let work = space.get(&numbers.part).ok_or_else(no_level)?;
            let node = last_by_id(styles, sheet.root, holder);
            let own = node.and_then(|n| own_num_id(styles, n));
            let theirs = own.and_then(|o| int(styles, o, "val"));
            let ok = own.is_none()
                || theirs.is_some_and(|t| definition_of(styles, sheet.root, work, numbers.root, t) == Some(definition));
            (node, ok)
        };
        if let (Some(node), true) = (node, theirs_ok) {
            drop_numbering(sheet.work(space)?, node, true);
        }
    }
    let work = sheet.work(space)?;
    let node = last_by_id(work, sheet.root, id).ok_or_else(|| EditError::Refused(format!("no style {id}")))?;
    let properties = crate::format::properties_of(work, node, "pPr", true, orders)
        .ok_or_else(|| EditError::no_element(sheet.part.clone()))?;
    if let Some(old) = work.find(properties, W, "numPr") {
        work.remove(old);
    }
    let numbering = make(work, "numPr", &[]);
    crate::marks::insert_in_order(work, properties, numbering, orders);
    if level != 0 {
        let made = make(work, "ilvl", &[("val", &level.to_string())]);
        work.append(numbering, made);
    }
    let made = make(work, "numId", &[("val", &num_id.to_string())]);
    work.append(numbering, made);
    if let Some(indent) = work.find(properties, W, "ind") {
        work.remove(indent);
    }
    Ok(unlinked)
}

/// Unlink style `id` (`unlink_style`): no level names it, its `w:numPr` goes (and its
/// `w:ind` where a level was linked), and a style still numbered through `basedOn` gets
/// `w:numId` 0. Whether anything changed.
///
/// # Errors
/// When a copy is gone.
pub(crate) fn unlink(space: &mut Space, sheet: &Sheet, numbers: Option<&Numbers>, id: &str) -> Result<bool> {
    let mut linked = false;
    if let Some(n) = numbers {
        let ties: Vec<Id> = {
            let styles = sheet.view(space)?;
            let work = space.get(&n.part).ok_or_else(|| EditError::no_part(n.part.clone()))?;
            let mut out = Vec::new();
            for (_, definition) in abstracts(work, n.root) {
                for (_, lvl) in definition_levels(styles, sheet.root, work, n.root, definition) {
                    if let Some(tied) = work
                        .find(lvl, W, "pStyle")
                        .filter(|t| work.get(*t, Some(W), "val") == Some(id))
                    {
                        out.push(tied);
                    }
                }
            }
            out
        };
        let work = numbers_work(space, n)?;
        for tied in ties {
            if work.parent(tied).is_some() {
                work.remove(tied);
                linked = true;
            }
        }
    }
    let orders = sheet.orders();
    let work = sheet.work(space)?;
    let node = last_by_id(work, sheet.root, id);
    let own = node.and_then(|n| own_num_id(work, n));
    if !linked && own.is_some_and(|o| work.get(o, Some(W), "val") == Some("0")) {
        return Ok(false);
    }
    let mut changed = node.is_some_and(|n| drop_numbering(work, n, linked));
    if let Some(node) = node {
        if style_list(space, sheet, numbers, id).is_some() {
            let work = sheet.work(space)?;
            let properties = crate::format::properties_of(work, node, "pPr", true, orders)
                .ok_or_else(|| EditError::no_element(sheet.part.clone()))?;
            let stop = make(work, "numPr", &[]);
            crate::marks::insert_in_order(work, properties, stop, orders);
            let zero = make(work, "numId", &[("val", "0")]);
            work.append(stop, zero);
            changed = true;
        }
    }
    Ok(linked || changed)
}

/// The paragraph styles numbered with style `id`'s `w:numPr` through their `basedOn`
/// chain (`inheriting_styles`), by id.
pub(crate) fn inheriting(space: &Space, sheet: &Sheet, numbers: Option<&Numbers>, id: &str) -> Vec<String> {
    let Some(styles) = space.get(&sheet.part) else {
        return Vec::new();
    };
    // docx-agent's dict of styles by id: the last of an id, in the first one's place
    let mut by_id: Vec<(String, Id)> = Vec::new();
    for node in children(styles, sheet.root, "style") {
        let key = styles.get(node, Some(W), "styleId").unwrap_or("").to_owned();
        match by_id.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = node,
            None => by_id.push((key, node)),
        }
    }
    let mut out = Vec::new();
    for (other, node) in &by_id {
        if other == id || styles.get(*node, Some(W), "type") != Some("paragraph") {
            continue;
        }
        let mut seen: Vec<String> = Vec::new();
        let mut current = Some(other.clone());
        while let Some(at) = current.clone().filter(|c| !c.is_empty() && !seen.contains(c)) {
            let Some((_, n)) = by_id.iter().find(|(k, _)| *k == at) else {
                break;
            };
            seen.push(at);
            if own_num_id(styles, *n).is_some() {
                break;
            }
            current = child_val(styles, *n, "basedOn").map(str::to_owned);
        }
        if current.as_deref() == Some(id) && style_list(space, sheet, numbers, other).is_some() {
            out.push(other.clone());
        }
    }
    out
}

/// The list `name` names: a number the document has, or a paragraph style in a list
/// (`list_style`), by `numId`.
pub(crate) fn list_style(space: &Space, sheet: &Sheet, numbers: Option<&Numbers>, style: &str) -> Option<i64> {
    let styles = space.get(&sheet.part)?;
    let mut current = Some(style.to_owned());
    let mut seen: Vec<String> = Vec::new();
    while let Some(at) = current.take().filter(|c| !c.is_empty() && !seen.contains(c)) {
        let node = last_by_id(styles, sheet.root, &at)?;
        if let Some(found) = own_num_id(styles, node) {
            let _ = numbers;
            return int(styles, found, "val").filter(|n| *n != 0);
        }
        current = child_val(styles, node, "basedOn").map(str::to_owned);
        seen.push(at);
    }
    None
}

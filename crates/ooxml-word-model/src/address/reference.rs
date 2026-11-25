//! Owns: the address resolver's entry points as they read an address's text before it was
//! read once ([`super::super::parsed::Address`]), frozen as the reference the typed ones are
//! compared with (lane ph2-types6, PR 7): every address of the gates' cases, on its
//! document, refused or resolved the same.
//!
//! Must not: be used but by tests.

use super::*;

/// `address` with a heading's range (`h:p:A`, or `h:` at a span's end) as the span of
/// blocks it is; any other address unchanged.
///
/// # Errors
/// [`heading`]'s.
pub(crate) fn expand_heading(docx: &Docx, address: &str) -> Result<String, AddressError> {
    if !address.contains(HEADING) {
        return Ok(address.to_owned());
    }
    let Some((first, last)) = address.split_once("..") else {
        if address.starts_with(HEADING) {
            let h = heading(docx, address)?;
            return Ok(if h.last == h.id {
                h.id
            } else {
                format!("{}..{}", h.id, h.last)
            });
        }
        return Ok(address.to_owned());
    };
    let (first, last) = (first.trim(), last.trim());
    if !(first.starts_with(HEADING) || last.starts_with(HEADING)) {
        return Ok(address.to_owned());
    }
    let first = if first.starts_with(HEADING) {
        heading(docx, first)?.id
    } else {
        first.to_owned()
    };
    let last = if last.starts_with(HEADING) {
        heading(docx, last)?.last
    } else {
        last.to_owned()
    };
    Ok(format!("{first}..{last}"))
}

/// The part and paragraphs (document order, tables' cells included) that `address` holds
/// -- any form: a story, a block or span, a row, a cell, a section, a bookmark, a text
/// range, a content control (docx-agent's `_scope_paragraphs`).
///
/// # Errors
/// What names nothing, or is malformed.
pub(crate) fn scope_paragraphs<'d>(docx: &'d Docx, address: &str) -> Result<Paragraphs<'d>, AddressError> {
    if address.trim().is_empty() {
        return Err(AddressError::invalid(
            format!("an address is empty: give a block id (p:..., t:...), a cell (t:X/c1,1), a section (s:1), a bookmark or a span ({SPAN_SYNTAX})"),
            Vec::new(),
        ));
    }
    let address = expand_heading(docx, address)?;
    let address = address.as_str();
    if let Some(part) = docx.story(address) {
        return Ok((part, (0..part.ids.paragraphs().len()).collect()));
    }
    if let Some((first, last)) = span_ends(address)? {
        return span_paragraphs(docx, first, last);
    }
    if is_text_range(address) {
        let range = text_range(docx, address)?;
        return range_paragraphs(docx, &range);
    }
    if address.starts_with("s:") || address.starts_with("s@") {
        let id = section(docx, address)?;
        let body = docx
            .body()
            .ok_or_else(|| AddressError::not_found("no body".to_owned(), Vec::new()))?;
        return Ok((body, section_paragraphs(docx, &id)));
    }
    if address.starts_with("bm:") || !address.contains([':', '@', '/', '#']) {
        let found = bookmark(docx, address)?;
        let range = bookmark_range(docx, &found)?;
        return range_paragraphs(docx, &range);
    }
    if address.starts_with("cc:") || address.starts_with("cc@") {
        let controls = crate::docx::controls::controls(docx);
        let found = control(&controls, address)?;
        let part = docx
            .part(&found.part)
            .ok_or_else(|| AddressError::not_found(format!("no part {}", found.part), Vec::new()))?;
        let inner = paragraphs_in(part, &[&found.path]);
        if !inner.is_empty() {
            return Ok((part, inner));
        }
        let holder = (1..found.path.len())
            .rev()
            .find_map(|n| found.path.get(..n).and_then(|p| part.ids.paragraph_at(p)));
        return Ok((part, holder.map(|p| vec![p.index]).unwrap_or_default()));
    }
    if let Some(found) = in_table(docx, address)? {
        return Ok((found.part, cell_paragraphs(&found)));
    }
    let (part, entry) = resolve(docx, address)?;
    match entry {
        Entry::Paragraph(k) => Ok((part, vec![k])),
        Entry::Table(t) => {
            let path = part.ids.tables().get(t).map(|t| t.path.clone()).unwrap_or_default();
            Ok((part, paragraphs_in(part, &[&path])))
        }
    }
}

/// Every address the gates' cases give, with the document each runs on: the frozen cases
/// (`crates/ooxml-oracle/cases`), each string of an address-taking argument, a place's
/// address too (`after:p:A` gives `p:A`).
fn corpus_addresses() -> Vec<(std::path::PathBuf, Vec<String>)> {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};
    const KEYS: [&str; 17] = [
        "target", "targets", "at", "table", "range", "after", "before", "section", "scope", "block", "blocks",
        "within", "id", "bookmark", "ref", "start", "address",
    ];
    fn files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files(&path, out);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                out.push(path);
            }
        }
    }
    fn strings(value: &serde_json::Value, key: &str, out: &mut BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    strings(v, k, out);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| strings(v, key, out)),
            serde_json::Value::String(text) if KEYS.contains(&key) => {
                out.insert(text.clone());
                for kind in ["after:", "before:", "replace:", "end:"] {
                    if let Some(rest) = text.strip_prefix(kind) {
                        out.insert(rest.to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    let oracle = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ooxml-oracle");
    let cases = oracle.join("cases");
    let mut paths = Vec::new();
    files(&cases, &mut paths);
    let mut by_document: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for path in paths {
        let Ok(relative) = path.strip_prefix(&cases) else {
            continue;
        };
        let own = relative
            .to_str()
            .and_then(|r| r.strip_suffix(".jsonl"))
            .filter(|r| !r.ends_with("_made"))
            .map(|r| oracle.join(r));
        for line in std::fs::read_to_string(&path).unwrap_or_default().lines() {
            let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let case = &entry["case"];
            let input = case["input"]
                .as_str()
                .and_then(|i| i.strip_prefix("$INPUT/"))
                .map(|i| cases.join("inputs").join(i));
            let Some(document) = input.or_else(|| own.clone()) else {
                continue;
            };
            strings(case, "", by_document.entry(document).or_default());
        }
    }
    by_document
        .into_iter()
        .map(|(document, addresses)| (document, addresses.into_iter().collect()))
        .collect()
}

/// What a resolver answered, comparable: the part and paragraphs, or the refusal.
fn scoped(found: Result<Paragraphs<'_>, AddressError>) -> Result<(String, Vec<usize>), AddressError> {
    found.map(|(part, indexes)| (part.part.clone(), indexes))
}

/// What a range selects, comparable: each selection's story, cell flag and Markdown.
fn selected(
    docx: &Docx,
    select: impl FnOnce(
        &mut crate::markdown::read::Reader<'_>,
    ) -> Result<Vec<crate::markdown::select::Selected>, AddressError>,
) -> Result<Vec<String>, AddressError> {
    use crate::markdown::select::Selected;
    let mut reader = crate::markdown::read::Reader::new(docx, crate::markdown::ViewName::Final);
    let found = select(&mut reader)?;
    Ok(found
        .into_iter()
        .map(|s| match s {
            Selected::Notes(part) => format!("notes {part}"),
            Selected::Records { story, records, cell } => {
                let blocks = crate::markdown::read::build(&mut reader, &records, false);
                format!("{story:?} {cell} {}", crate::markdown::render::render(&blocks))
            }
        })
        .collect())
}

#[test]
fn every_address_of_the_cases_resolves_and_is_refused_as_before() {
    use super::super::parsed::{Address, Place, PlaceKind, Shape};
    let mut compared = 0;
    let mut differences = Vec::new();
    for (document, addresses) in corpus_addresses() {
        let Ok(bytes) = std::fs::read(&document) else { continue };
        let Ok(package) = ooxml_opc::Package::open(Arc::from(bytes), &ooxml_base::Limits::default()) else {
            continue;
        };
        let Ok(docx) = Docx::open(&package) else { continue };
        for raw in &addresses {
            let address = Address::parse(raw);
            let mut differ = |what: &str| differences.push(format!("{}: {raw:?}: {what}", document.display()));
            let old = expand_heading(&docx, raw);
            let new = super::expand_heading(&docx, &address).map(|a| a.as_str().to_owned());
            if old != new {
                differ("expand_heading");
            }
            if scoped(scope_paragraphs(&docx, raw)) != scoped(super::scope_paragraphs(&docx, &address)) {
                differ("scope_paragraphs");
            }
            let old = selected(&docx, |r| crate::markdown::select::reference::select(r, Some(raw)));
            let new = selected(&docx, |r| crate::markdown::select::select(r, Some(&address)));
            if old != new {
                differ("select");
            }
            // a place as `_locate` split it
            let (kind, rest) = match raw.split_once(':') {
                Some((k, r)) if matches!(k, "after" | "before" | "replace" | "end") => (k, r),
                _ if raw == "end" => ("end", ""),
                _ => ("at", raw.as_str()),
            };
            let same = match Place::parse(raw).kind() {
                PlaceKind::End(story) => kind == "end" && story.as_deref().unwrap_or("") == rest,
                PlaceKind::After(a) => kind == "after" && a.as_str() == rest,
                PlaceKind::Before(a) => kind == "before" && a.as_str() == rest,
                PlaceKind::Replace(a) => kind == "replace" && a.as_str() == rest,
                PlaceKind::At(a) => kind == "at" && a.as_str() == rest,
            };
            if !same {
                differ("place");
            }
            // the forms step 8's commands ask of a typed address, as they sniffed the text
            let sniffed = [
                (Shape::Dots, raw.contains("..")),
                (Shape::Text, is_text_range(raw)),
                (Shape::Section, raw.starts_with("s:") || raw.starts_with("s@")),
                (Shape::Control, raw.starts_with("cc:") || raw.starts_with("cc@")),
            ];
            for (shape, old) in sniffed {
                if address.has(shape) != old {
                    differ(&format!("{shape:?}"));
                }
            }
            let ends = |e: Result<Option<super::super::syntax::Ends<'_>>, AddressError>| {
                e.map(|o| o.map(|(a, b)| (a.map(str::to_owned), b.map(str::to_owned))))
            };
            if ends(span_ends(raw)) != ends(address.span_ends()) {
                differ("span_ends");
            }
            compared += 1;
        }
    }
    assert!(
        differences.is_empty(),
        "{} differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
    assert!(compared > 1000, "{compared} addresses compared");
}

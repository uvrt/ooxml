//! `word-ids <manifest.json>`: each document's block ids and id ledger as JSON, in the form
//! `python/word_reads_diff.py` writes docx-agent's (the ids of every story part, the
//! `docx-ids.1.` record of the document as opened, the next values of each part's
//! sequence; with a state, the same after the document is opened with it).
//!
//! The manifest is a JSON list of `{"file": path, "state": string or null}`. With
//! `--renamed <file.docx>...`, each document's ids an unstamped reading named otherwise
//! ([`renamed_at_open`]).
//!
//! Must not: be used by any library; it is a test instrument.

use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_word_model::docx::sections::section_id;
use ooxml_word_model::ids::{block_and_drawing_ids, load_state, record_of, story_name, Ledger, PartIds, Versions};
use ooxml_word_model::parts::{main_part, parts};
use ooxml_word_model::WordPackage;
use ooxml_xml::Tree;
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// Run over a manifest: one JSON line per document on stdout.
///
/// # Errors
/// A message when the manifest or a document cannot be read.
pub fn run(args: &[String]) -> Result<(), String> {
    if args.first().map(String::as_str) == Some("--renamed") {
        for file in args.get(1..).unwrap_or_default() {
            let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            let renamed = renamed_at_open(bytes).unwrap_or_else(|e| json!({"error": e}));
            println!("{}", json!({"file": file, "renamed": renamed}));
        }
        return Ok(());
    }
    let manifest = args
        .first()
        .ok_or("usage: word-ids <manifest.json> | word-ids --renamed <file.docx>...")?;
    let text = std::fs::read_to_string(manifest).map_err(|e| e.to_string())?;
    let items: Vec<Value> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    for item in items {
        let file = item
            .get("file")
            .and_then(Value::as_str)
            .ok_or("an item without a file")?;
        let state = item.get("state").and_then(Value::as_str);
        let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
        let out = ids_of(bytes, state).unwrap_or_else(|e| json!({"error": e}));
        println!("{}", json!({"file": file, "ids": out}));
    }
    Ok(())
}

/// One document's ids, ledger record and next values, as a session opens it: every story
/// block stamped in memory ([`WordPackage::open`]), the record's bytes the ids as saved.
///
/// # Errors
/// A message when the package cannot be read.
pub fn ids_of(bytes: Vec<u8>, state: Option<&str>) -> Result<Value, String> {
    let package = Package::open(Arc::from(bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let (package, pool) = WordPackage::open(package);
    let main = main_part(&package).map_err(|e| e.to_string())?.ok_or("no main part")?;
    let names = parts(&package).map_err(|e| e.to_string())?;
    let mut trees: Vec<(String, Tree)> = Vec::new();
    let mut saved: Vec<Tree> = Vec::new();
    for name in names {
        if let Some(tree) = package.tree(&name).map_err(|e| e.to_string())? {
            trees.push((name.clone(), tree));
        }
        saved.extend(package.saved_tree(&name).map_err(|e| e.to_string())?);
    }
    let roots: Vec<&ooxml_xml::Element> = trees.iter().map(|(_, t)| t.root().as_ref()).collect();
    let saved: Vec<&ooxml_xml::Element> = saved.iter().map(|t| t.root().as_ref()).collect();
    let index: Vec<Value> = trees
        .iter()
        .map(|(part, tree)| index_json(part, &story_name(&main, part), tree))
        .collect();
    let mut ledger = Ledger {
        pool: Some(pool),
        ..Ledger::default()
    };
    let mut versions = Versions { version: 0, last: 0 };
    if let Some(state) = state {
        let mut loaded = load_state(state, &saved).map_err(|e| e.to_string())?;
        // an id the state retired that the open gave again is live
        let (live, _) = block_and_drawing_ids(&roots);
        loaded.gone.retain(|value, _| !live.contains(value));
        versions = loaded.versions.clone();
        let mut pool = ledger.pool.take().unwrap_or_default();
        pool.extend(loaded.pool.values().iter().copied());
        for (seed, k) in loaded.pool.cursors() {
            pool.set_cursor(seed, *k);
        }
        ledger.restore(loaded);
        ledger.pool = Some(pool);
    }
    ledger.note(&roots, versions.version);
    let record = record_of(&ledger, &saved, &versions, &|_| None);
    let mut generated = Map::new();
    for (part, _) in &trees {
        let made = ledger.pool(&roots, versions.version).generate(part, 3);
        generated.insert(part.clone(), json!(made));
    }
    Ok(json!({"parts": index, "record": record, "generated": generated}))
}

/// Each block id a reading of the document without stamping names (positional `p@body/3`,
/// `t@body/0`, a row `t:X/r1`, a repeat `p:X#1`, an out-of-range paraId), and each section
/// named after such a paragraph (`s@body/3`), that the document opened -- every story block
/// stamped in memory ([`WordPackage::open`]) -- names otherwise: `{old: new}`. What
/// the stamp-on-open change (user decision 2026-11-26) re-targets the gates' cases and the
/// host goldens by (`python/stamp_on_open.py`).
///
/// # Errors
/// A message when the package cannot be read.
pub fn renamed_at_open(bytes: Vec<u8>) -> Result<Value, String> {
    let raw = Package::open(Arc::from(bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let (word, _) = WordPackage::open(raw.clone());
    let Some(main) = main_part(&raw).map_err(|e| e.to_string())? else {
        return Ok(json!({}));
    };
    let names = |ids: PartIds| {
        let mut out: Vec<String> = ids.paragraphs().iter().map(|p| p.id).collect();
        for table in ids.tables().iter() {
            out.push(table.id.clone());
            out.extend(table.rows.iter().map(|(_, r)| r.clone()));
        }
        out
    };
    let mut renamed = Map::new();
    for part in parts(&raw).map_err(|e| e.to_string())? {
        let (Some(old), Some(new)) = (
            raw.tree(&part).map_err(|e| e.to_string())?,
            word.tree(&part).map_err(|e| e.to_string())?,
        ) else {
            continue;
        };
        let story = story_name(&main, &part);
        let (old, new) = (
            names(PartIds::new(&story, old.root())),
            names(PartIds::new(&story, new.root())),
        );
        if old.len() != new.len() {
            return Err(format!("{part}: {} blocks unstamped, {} stamped", old.len(), new.len()));
        }
        for (old, new) in old.into_iter().zip(new) {
            if old != new {
                // a section is named after the paragraph that ends it
                if !old.contains("t@") && !old.contains("t:") && !old.contains("tr:") {
                    renamed.insert(section_id(&old), Value::from(section_id(&new)));
                }
                renamed.insert(old, Value::from(new));
            }
        }
    }
    Ok(Value::Object(renamed))
}

fn index_json(part: &str, story: &str, tree: &Tree) -> Value {
    let ids = PartIds::new(story, tree.root());
    let paragraphs: Vec<Value> = ids
        .paragraphs()
        .iter()
        .map(|p| json!([p.id, format!("{:?}", p.state).to_lowercase()]))
        .collect();
    let tables: Vec<Value> = ids
        .tables()
        .iter()
        .map(|t| json!([t.id, t.rows.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>()]))
        .collect();
    json!({"part": part, "story": story, "paragraphs": paragraphs, "tables": tables})
}

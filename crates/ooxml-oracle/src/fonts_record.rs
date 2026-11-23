//! Owns: a fonts record -- the JSON the layout and paint benchmarks take (`layout-bench`,
//! `paint-bench`): the font folders (`dirs`, `[location, path]` pairs, `bundle` for Word's
//! own), `substitutes`, `recorded_faces`, `fallbacks`, the document's `font_table` and its
//! `embedded` faces -- read as Word's fonts ([`fonts_of`]). The form docx2svg's former
//! differential tools wrote; the benchmarks keep it.
//!
//! Must not: lay out or measure.

use crate::disk::DiskFonts;
use ooxml_fonts::{EmbeddedPart, Folder, FontTable, FontTableEntry, Location, WordFontOptions, WordFonts};
use serde_json::Value;
use std::sync::Arc;

/// Word's fonts as a fonts record names them.
pub fn fonts_of(record: &Value, package: Option<&[u8]>) -> Result<WordFonts, String> {
    let (folders, embedded, table, options) = font_inputs(record, package)?;
    Ok(WordFonts::new(Arc::new(DiskFonts), &folders, &embedded, table, options))
}

/// What a fonts record says: the folders, the embedded faces, the font table, the options.
type FontInputs = (Vec<Folder>, Vec<ooxml_fonts::EmbeddedPart>, FontTable, WordFontOptions);

fn font_inputs(record: &Value, package: Option<&[u8]>) -> Result<FontInputs, String> {
    let mut folders = Vec::new();
    for pair in record.get("dirs").and_then(Value::as_array).into_iter().flatten() {
        let location = if pair.get(0).and_then(Value::as_str) == Some("bundle") {
            Location::WordBundle
        } else {
            Location::Folder
        };
        folders.push(Folder {
            location,
            path: pair.get(1).and_then(Value::as_str).unwrap_or_default().to_owned(),
        });
    }
    let substitutes = record
        .get("substitutes")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                .collect()
        })
        .unwrap_or_default();
    let options = WordFontOptions {
        substitutes,
        recorded_faces: record.get("recorded_faces").and_then(Value::as_bool).unwrap_or(true),
        fallbacks: record.get("fallbacks").and_then(Value::as_bool).unwrap_or(true),
    };
    let mut table = FontTable::new();
    for (name, entry) in record
        .get("font_table")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let text = |k: usize| entry.get(k).and_then(Value::as_str).map(str::to_owned);
        table.insert(
            name.clone(),
            FontTableEntry {
                family: text(0),
                pitch: text(1),
            },
        );
    }
    let embedded = embedded_parts(record, package)?;
    Ok((folders, embedded, table, options))
}

fn embedded_parts(record: &Value, package: Option<&[u8]>) -> Result<Vec<EmbeddedPart>, String> {
    let list = record
        .get("embedded")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if list.is_empty() {
        return Ok(Vec::new());
    }
    let package = package.ok_or("embedded faces but no package")?;
    let archive =
        ooxml_zip::Archive::read(Arc::from(package), &ooxml_zip::Limits::default()).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in list {
        let text = |k: usize| row.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        let part = text(2);
        let Some(entry) = archive.entries().iter().find(|e| e.meta().name == part) else {
            continue;
        };
        let bytes = archive.read_entry(entry).map_err(|e| e.to_string())?;
        out.push(EmbeddedPart {
            family: text(0),
            style: text(1),
            key: text(3),
            bytes,
        });
    }
    Ok(out)
}

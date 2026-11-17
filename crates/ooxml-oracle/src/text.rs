//! Owns: the text engine's differential -- each document's records measured with the
//! Rust engine over the fonts docx2svg found, every piece and line compared, and every
//! difference classified by the first field that differs.
//!
//! Must not: change what is compared to make it pass.

use crate::disk::DiskFonts;
use crate::glyphs::{Coverage, WordGlyphs};
use crate::records::{add_formats, expected, paragraph, Expected, ExpectedPiece, Formats};
use crate::replay::ReplayGlyphs;
use ooxml_fonts::{EmbeddedPart, Folder, FontTable, FontTableEntry, Location, WordFontOptions, WordFonts};
use ooxml_text::{measure, Geometry, Glyphs, Paragraph, PieceKind};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One document's records.
#[derive(Debug, Default)]
pub struct DocumentRecords {
    /// The document's path.
    pub path: String,
    /// Its paragraph records, in order.
    pub paragraphs: Vec<Value>,
    /// Its fonts record.
    pub fonts: Option<Value>,
    /// docx2svg's substitutions report.
    pub report: Option<Value>,
    /// Its format records, merged.
    pub formats: Formats,
    /// Why docx2svg refused the document, if it did.
    pub refused: Option<String>,
}

/// A difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// The document.
    pub document: String,
    /// The paragraph's number among the document's records.
    pub paragraph: usize,
    /// Its text, shortened.
    pub text: String,
    /// What differs: the first field.
    pub cause: String,
    /// The detail.
    pub detail: String,
}

/// The differential's result.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Paragraph records compared.
    pub compared: usize,
    /// Of them, measured (not unmeasurable) on both sides.
    pub measured: usize,
    /// Of them, unmeasurable on both sides.
    pub unmeasurable: usize,
    /// Pieces compared.
    pub pieces: usize,
    /// Lines compared.
    pub lines: usize,
    /// Every difference.
    pub differences: Vec<Difference>,
    /// Report differences (substitutions, missing faces, glyph fallbacks).
    pub report_differences: Vec<String>,
    /// Time spent measuring and breaking, the engine's own (not reading records or fonts).
    pub engine_time: Duration,
    /// Documents read.
    pub documents: usize,
}

/// Word's fonts as a document's fonts record (`text_differential.py`'s) found them.
pub fn fonts_of(record: &Value, package: Option<&[u8]>) -> Result<WordFonts, String> {
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
    Ok(WordFonts::new(Arc::new(DiskFonts), &folders, &embedded, table, options))
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

fn short(paragraph: &Paragraph) -> String {
    let text: String = paragraph.runs.iter().map(|r| r.text.as_str()).collect();
    text.chars().take(60).collect()
}

fn kind_code(kind: PieceKind) -> &'static str {
    match kind {
        PieceKind::Glyph => "g",
        PieceKind::Space => "s",
        PieceKind::Tab => "t",
        PieceKind::Break => "b",
        PieceKind::SoftHyphen => "h",
        PieceKind::Object => "o",
    }
}

/// The first field in which a Rust piece differs from docx2svg's, or `None`.
fn piece_difference(ours: &ooxml_text::Piece, theirs: &ExpectedPiece) -> Option<(&'static str, String)> {
    let ch = (ours.ch != '\u{0}').then_some(ours.ch);
    let checks: [(&'static str, bool, String); 9] = [
        (
            "kind",
            kind_code(ours.kind) == theirs.kind,
            format!("{} vs {}", kind_code(ours.kind), theirs.kind),
        ),
        ("char", ch == theirs.ch, format!("{:?} vs {:?}", ch, theirs.ch)),
        (
            "width",
            ours.width == theirs.width,
            format!("{:?}: {} vs {}", ours.ch, ours.width, theirs.width),
        ),
        (
            "kern",
            theirs.kern.is_none_or(|k| k == ours.kern_next),
            format!("{:?}: {} vs {:?}", ours.ch, ours.kern_next, theirs.kern),
        ),
        (
            "source",
            ours.source == theirs.source,
            format!("{} vs {}", ours.source, theirs.source),
        ),
        (
            "hyphen",
            ours.hyphen == theirs.hyphen,
            format!("{} vs {}", ours.hyphen, theirs.hyphen),
        ),
        ("run", ours.run == theirs.run, format!("{} vs {}", ours.run, theirs.run)),
        (
            "auto",
            ours.auto == theirs.auto && ours.proof_end == theirs.proof_end,
            format!("{} vs {}", ours.auto, theirs.auto),
        ),
        (
            "height",
            ours.height == theirs.height,
            format!("{} vs {}", ours.height, theirs.height),
        ),
    ];
    checks
        .into_iter()
        .find(|(_, same, _)| !same)
        .map(|(cause, _, detail)| (cause, detail))
}

/// Compare one paragraph; the difference's cause and detail, if any.
fn compare(
    glyphs: &dyn Glyphs,
    input: &(Paragraph, Geometry, bool),
    expected: &Expected,
    outcome: &mut Outcome,
) -> Option<(String, String)> {
    let started = Instant::now();
    let ours = measure(glyphs, &input.0, &input.1, input.2);
    outcome.engine_time += started.elapsed();
    match (ours, expected) {
        (Err(_), Expected::Error(_)) => {
            outcome.unmeasurable += 1;
            None
        }
        (Err(e), Expected::Measured(..)) => Some(("unmeasurable in Rust only".into(), e.to_string())),
        (Ok(_), Expected::Error(e)) => Some(("unmeasurable in docx2svg only".into(), e.clone())),
        (Ok(m), Expected::Measured(pieces, lines)) => {
            outcome.measured += 1;
            outcome.pieces += pieces.len();
            outcome.lines += lines.len();
            if m.pieces.len() != pieces.len() {
                let at = m
                    .pieces
                    .iter()
                    .zip(pieces)
                    .position(|(a, b)| piece_difference(a, b).is_some());
                return Some((
                    "piece count".into(),
                    format!("{} vs {}, first differing at {at:?}", m.pieces.len(), pieces.len()),
                ));
            }
            for (k, (a, b)) in m.pieces.iter().zip(pieces).enumerate() {
                if let Some((cause, detail)) = piece_difference(a, b) {
                    return Some((format!("piece {cause}"), format!("piece {k}: {detail}")));
                }
            }
            (m.lines != *lines).then(|| ("lines".into(), format!("{:?} vs {:?}", m.lines, lines)))
        }
    }
}

/// Measure and compare one document's records.
pub fn run_document(document: &DocumentRecords, outcome: &mut Outcome) -> Result<(), String> {
    outcome.documents += 1;
    let installed = match &document.fonts {
        Some(fonts) => {
            let package = std::fs::read(&document.path).ok();
            Some(WordGlyphs::new(fonts_of(fonts, package.as_deref())?))
        }
        None => None,
    };
    for (number, record) in document.paragraphs.iter().enumerate() {
        let input = paragraph(&document.formats, record)?;
        let wanted = expected(record)?;
        let replay = record.get("glyphs").map(ReplayGlyphs::new);
        let glyphs: &dyn Glyphs = match (&replay, &installed) {
            (Some(replay), _) => replay,
            (None, Some(installed)) => installed,
            (None, None) => return Err(format!("{}: a paragraph without fonts or answers", document.path)),
        };
        outcome.compared += 1;
        if let Some((cause, detail)) = compare(glyphs, &input, &wanted, outcome) {
            outcome.differences.push(Difference {
                document: document.path.clone(),
                paragraph: number,
                text: short(&input.0),
                cause,
                detail,
            });
        }
    }
    if let (Some(report), Some(installed)) = (&document.report, &installed) {
        compare_report(&document.path, report, &installed.coverage(), outcome);
    }
    Ok(())
}

fn compare_report(path: &str, report: &Value, coverage: &Coverage, outcome: &mut Outcome) {
    use ooxml_fonts::Grade;
    let mut theirs: BTreeMap<String, String> = BTreeMap::new();
    for (family, s) in report
        .get("substitutions")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        theirs.insert(
            family.clone(),
            s.get("substitute")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        );
    }
    for family in report
        .get("missing")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        theirs.insert(family.to_owned(), "(missing)".into());
    }
    let mut ours: BTreeMap<String, String> = BTreeMap::new();
    for (family, grade) in &coverage.grades {
        let named = match grade {
            Grade::Substituted(s) => s.substitute.clone(),
            Grade::Recorded => "recorded metrics".into(),
            Grade::Missing => "(missing)".into(),
            Grade::Installed | Grade::Embedded => continue,
        };
        ours.insert(family.clone(), named);
    }
    if ours != theirs {
        outcome
            .report_differences
            .push(format!("{path}: substitutions {ours:?} vs {theirs:?}"));
    }
    let mut fallbacks: BTreeMap<(String, char), String> = BTreeMap::new();
    for row in report
        .get("glyph_fallbacks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |k: usize| row.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        if let Some(ch) = text(1).chars().next() {
            fallbacks.insert((text(0), ch), text(2));
        }
    }
    if fallbacks != coverage.glyph_fallbacks {
        outcome.report_differences.push(format!(
            "{path}: glyph fallbacks {:?} vs {fallbacks:?}",
            coverage.glyph_fallbacks
        ));
    }
}

/// Read a records stream, handing each document to `each` as it completes.
pub fn read_records(
    text: impl std::io::BufRead,
    mut each: impl FnMut(DocumentRecords) -> Result<(), String>,
) -> Result<(), String> {
    let mut current: Option<DocumentRecords> = None;
    for line in text.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let value: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if let Some(document) = value.get("document") {
            if let Some(done) = current.take() {
                each(done)?;
            }
            let path = document
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            current = Some(DocumentRecords {
                path,
                ..DocumentRecords::default()
            });
            continue;
        }
        let Some(doc) = current.as_mut() else { continue };
        if let Some(list) = value.get("formats") {
            add_formats(&mut doc.formats, list)?;
        } else if let Some(p) = value.get("paragraph") {
            doc.paragraphs.push(p.clone());
        } else if let Some(f) = value.get("fonts") {
            doc.fonts = Some(f.clone());
        } else if let Some(r) = value.get("report") {
            doc.report = Some(r.clone());
        } else if let Some(r) = value.get("refused").and_then(Value::as_str) {
            doc.refused = Some(r.to_owned());
        }
    }
    if let Some(done) = current.take() {
        each(done)?;
    }
    Ok(())
}

//! Owns: what `open_document` says of a Word document (docx-agent's `tools/format.py`
//! `summary`): its title, how many paragraphs, tables, headings, comments and revision
//! records it has, and whether it is a template -- cheap facts, no layout.
//!
//! Must not: lay out, or read more than those counts need.

use super::package::Reader;
use super::{opc, PROPERTIES};
use ooxml_base::Cancel;
use ooxml_opc::Package;
use ooxml_tools::ToolError;
use ooxml_word_edit::FileKind;
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::outline::{body_blocks, headings};
use ooxml_word_model::docx::revisions::records;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::parts::main_part;
use serde_json::{json, Value};

/// The summary of the Word document whose version is `package` (read as `docx`):
/// `{"title", "paragraphs", "tables", "headings", "comments", "revisions", "template"}`, as
/// docx-agent's format gives it -- the title `null` where the core properties set none, the
/// paragraphs and tables the body's (those in its tables, controls and text boxes too), the
/// comments every one (replies too).
///
/// # Errors
/// `internal` when a part it reads is not XML or the package has no main part; `timeout`
/// when `cancel` asks to stop.
pub fn summary(package: &Package, docx: &Docx, cancel: &dyn Cancel) -> Result<Value, ToolError> {
    let reader = Reader { package, cancel };
    let main = main_part(package)
        .map_err(opc)?
        .ok_or_else(|| ToolError::internal("not a Word package: no main document part", "summary:main"))?;
    let title_key = PROPERTIES[0].0;
    let title = reader.properties()?.remove(title_key).unwrap_or(Value::Null);
    let (paragraphs, tables) = docx
        .body()
        .map_or((0, 0), |body| (body.ids.paragraphs().len(), body.ids.tables().len()));
    let found = controls(docx);
    let heading_count = headings(&body_blocks(docx, &found)).len();
    let comments = reader.comment_records(&main)?.len();
    let revisions = records(docx).len();
    let template = matches!(
        FileKind::of(package).map_err(|e| ToolError::internal(e.to_string(), "summary:kind"))?,
        Some(FileKind::Dotx | FileKind::Dotm)
    );
    Ok(json!({
        "title": title, "paragraphs": paragraphs, "tables": tables, "headings": heading_count,
        "comments": comments, "revisions": revisions, "template": template,
    }))
}

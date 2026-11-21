//! Owns: the problems only a save finds (docx-agent's `validate.extension_problems`): given
//! the file name the document is written to, whether the main part's content type is the
//! one its extension names (`content-type-extension`: Word refuses a `.docx` whose main
//! part is a template's, and a `.dotx` whose main part is a document's) and whether a
//! `.docx` or `.dotx` carries a VBA project (`macros-in-macro-free-file`); and Word's
//! package kinds ([`WordKind`]).
//!
//! Must not: change the package (writing the kind's content type is the save's job), or
//! check an extension that names no Word kind.

use super::Problem;
use crate::{Result, WordDoc};
use ooxml_opc::CONTENT_TYPES_PART;

/// One of Word's package kinds (`ooxml_common.kinds`' Word rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordKind {
    /// `docx`, `docm`, `dotx` or `dotm`: its extension without the dot.
    pub name: &'static str,
    /// Whether it may carry a VBA project.
    pub macro_enabled: bool,
    /// The content type its main part declares.
    pub content_type: &'static str,
}

/// Word's kinds, as ECMA-376 and Office's macro-enabled extensions name them.
pub const WORD_KINDS: [WordKind; 4] = [
    WordKind {
        name: "docx",
        macro_enabled: false,
        content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
    },
    WordKind {
        name: "docm",
        macro_enabled: true,
        content_type: "application/vnd.ms-word.document.macroEnabled.main+xml",
    },
    WordKind {
        name: "dotx",
        macro_enabled: false,
        content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml",
    },
    WordKind {
        name: "dotm",
        macro_enabled: true,
        content_type: "application/vnd.ms-word.template.macroEnabledTemplate.main+xml",
    },
];

/// The relationships from the main part to a VBA project or what belongs to one
/// (docx-agent's `_MACRO_RELATIONSHIPS`).
const MACRO_RELATIONSHIPS: [&str; 3] = [
    "http://schemas.microsoft.com/office/2006/relationships/vbaProject",
    "http://schemas.microsoft.com/office/2006/relationships/keyMapCustomizations",
    "http://schemas.microsoft.com/office/2006/relationships/wordVbaData",
];

impl WordKind {
    /// The kind `target`'s extension names, case aside (`os.path.splitext`'s: the last dot
    /// of the file name, leading dots not counted); `None` for another extension.
    #[must_use]
    pub fn of_target(target: &str) -> Option<&'static WordKind> {
        let name = target.rsplit('/').next().unwrap_or(target);
        let name = name.trim_start_matches('.');
        let (_, extension) = name.rsplit_once('.')?;
        let extension = extension.to_ascii_lowercase();
        WORD_KINDS.iter().find(|k| k.name == extension)
    }

    /// The kind whose main part declares `content_type`.
    #[must_use]
    pub fn of_content_type(content_type: Option<&str>) -> Option<&'static WordKind> {
        WORD_KINDS.iter().find(|k| Some(k.content_type) == content_type)
    }
}

/// The problems of `doc` written to `target` (a file name): its main part's content type
/// against the kind the extension names, and a VBA project in a kind that cannot carry
/// one. None for an extension that names no Word kind.
///
/// # Errors
/// When the version cannot be read as a Word document.
pub fn extension_problems(doc: &WordDoc, target: &str) -> Result<Vec<Problem>> {
    let Some(wanted) = WordKind::of_target(target) else {
        return Ok(Vec::new());
    };
    let docx = doc.docx()?;
    let package = doc.package();
    let types = package.content_types()?;
    let declared = types.of(&docx.main);
    let mut out = Vec::new();
    if declared != Some(wanted.content_type) {
        let is = match WordKind::of_content_type(declared) {
            Some(kind) => format!("a .{}'s", kind.name),
            None => "of no Word kind".to_owned(),
        };
        out.push(Problem {
            code: "content-type-extension".into(),
            part: CONTENT_TYPES_PART.into(),
            detail: format!(
                "a .{} file whose main part is {is} ({}): Word refuses it; save() writes {}",
                wanted.name,
                declared.unwrap_or("None"),
                wanted.content_type
            ),
        });
    }
    if !wanted.macro_enabled && has_macros(doc, &docx.main)? {
        out.push(Problem {
            code: "macros-in-macro-free-file".into(),
            part: docx.main.clone(),
            detail: format!("a .{} cannot carry a VBA project", wanted.name),
        });
    }
    Ok(out)
}

/// Whether the main part has a VBA project or what belongs to one (docx-agent's
/// `has_macros`).
fn has_macros(doc: &WordDoc, main: &str) -> Result<bool> {
    let rels = doc.package().relationships(main)?;
    let found = rels.iter().any(|r| MACRO_RELATIONSHIPS.contains(&r.rel_type.as_str()));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::WordKind;

    #[test]
    fn a_target_names_its_kind_as_splitext_reads_it() {
        let name = |t: &str| WordKind::of_target(t).map(|k| k.name);
        assert_eq!(name("report.docx"), Some("docx"));
        assert_eq!(name("dir.v2/Report.DOTM"), Some("dotm"));
        assert_eq!(name(".docx"), None);
        assert_eq!(name("notes.txt"), None);
        assert_eq!(name("docx"), None);
    }
}

//! Owns: a version written as a file of a Word kind, as docx-agent's `serialise` writes it
//! for `save_document`: refused when it holds macros the kind cannot carry, the main
//! part's content type set to the kind's (the rest of the package as it is), and
//! `app.xml`'s statistics refreshed where the caller gives them ([`app`]); and
//! [`clean`], the written file with what Word keeps but never draws taken out.
//!
//! Must not: change the version (the parts it writes differently are written in place of
//! the version's own, as the package's `replacements`), or decide whether the statistics
//! are due (the caller knows what changed since the document was opened).

mod app;
mod cleaning;
mod orphans;

pub use app::AppStatistics;
pub use cleaning::{clean, Cleaned};

use crate::content_types::retyped;
use crate::doc::ModelError;
use crate::{EditError, Result};
use ooxml_opc::{Package, CONTENT_TYPES_PART};
use ooxml_word_model::parts::{main_part, relationships};
use std::sync::Arc;

/// How ooxml-edit writes `[Content_Types].xml` and the parts it serialises itself.
const DECLARATION: &[u8] = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";

const OOXML: &str = "application/vnd.openxmlformats-officedocument.";
/// The relationships of a VBA project (or what belongs to one) from the main part.
const MACRO_RELATIONSHIPS: [&str; 3] = [
    "http://schemas.microsoft.com/office/2006/relationships/vbaProject",
    "http://schemas.microsoft.com/office/2006/relationships/keyMapCustomizations",
    "http://schemas.microsoft.com/office/2006/relationships/wordVbaData",
];

/// A Word file kind (its extension).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// A document.
    Docx,
    /// A document with macros.
    Docm,
    /// A template.
    Dotx,
    /// A template with macros.
    Dotm,
}

impl FileKind {
    /// Every kind.
    pub const ALL: [FileKind; 4] = [Self::Docx, Self::Docm, Self::Dotx, Self::Dotm];

    /// Its name, the extension without the dot.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Docm => "docm",
            Self::Dotx => "dotx",
            Self::Dotm => "dotm",
        }
    }

    /// The kind named `name` (an extension, any case).
    #[must_use]
    pub fn named(name: &str) -> Option<FileKind> {
        Self::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(name))
    }

    /// The main part's content type.
    #[must_use]
    pub fn content_type(self) -> String {
        match self {
            Self::Docx => format!("{OOXML}wordprocessingml.document.main+xml"),
            Self::Docm => "application/vnd.ms-word.document.macroEnabled.main+xml".to_owned(),
            Self::Dotx => format!("{OOXML}wordprocessingml.template.main+xml"),
            Self::Dotm => "application/vnd.ms-word.template.macroEnabledTemplate.main+xml".to_owned(),
        }
    }

    /// Whether it may carry macros.
    #[must_use]
    pub fn macro_enabled(self) -> bool {
        matches!(self, Self::Docm | Self::Dotm)
    }

    /// The kind of `package`, by its main part's content type (`None`: not one of these).
    ///
    /// # Errors
    /// When the package's relationships or content types are not XML.
    pub fn of(package: &Package) -> Result<Option<FileKind>> {
        let Some(main) = main_part(package).map_err(ModelError::from)? else {
            return Ok(None);
        };
        let types = package.content_types().map_err(ModelError::from)?;
        let Some(found) = types.of(&main) else {
            return Ok(None);
        };
        Ok(Self::ALL.into_iter().find(|k| k.content_type() == found))
    }
}

/// Whether the main part has a VBA project (or what belongs to one).
///
/// # Errors
/// When the package's relationships are not XML.
pub fn has_macros(package: &Package) -> Result<bool> {
    let Some(main) = main_part(package).map_err(ModelError::from)? else {
        return Ok(false);
    };
    Ok(relationships(package, &main)
        .map_err(ModelError::from)?
        .iter()
        .any(|r| MACRO_RELATIONSHIPS.contains(&r.rel_type.as_str())))
}

/// `package` written as a file of `kind`: refused when it holds macros `kind` cannot carry;
/// the main part's content type the kind's; `app.xml`'s statistics brought up to `app`
/// where given (`None`: as they are).
///
/// # Errors
/// [`EditError::Refused`] for the macros; when a part it reads is not XML, or the package
/// cannot be written.
pub fn serialise(package: &Package, kind: FileKind, app: Option<&AppStatistics>) -> Result<Vec<u8>> {
    if !kind.macro_enabled() && has_macros(package)? {
        return Err(EditError::Refused(format!(
            "the document holds macros, which a .{} cannot carry",
            kind.name()
        )));
    }
    let mut out = package.clone();
    if FileKind::of(package)? != Some(kind) {
        if let Some(main) = main_part(package).map_err(ModelError::from)? {
            if let Some(retyped) = retyped(package, &main, &kind.content_type())? {
                out = out.with_bytes(CONTENT_TYPES_PART, written(&retyped, CONTENT_TYPES_PART)?);
            }
        }
    }
    if let Some(stats) = app {
        if let Some((part, bytes)) = app::refreshed(package, stats)? {
            out = out.with_bytes(&part, bytes);
        }
    }
    Ok(out.save().map_err(ModelError::from)?)
}

/// The part's tree written as ooxml-edit writes a part it serialises itself: its
/// declaration, then the root (lxml's `tostring`).
fn written(package: &Package, part: &str) -> Result<Arc<[u8]>> {
    let tree = package
        .tree(part)
        .map_err(ModelError::from)?
        .ok_or_else(|| EditError::NoPart(part.to_owned()))?;
    let mut out = DECLARATION.to_vec();
    out.extend_from_slice(&tree.root().write());
    Ok(out.into())
}

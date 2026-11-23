//! Owns: `save_document` (docx-agent's handler of the shared tool): the version written as
//! the file asked for -- a .docx or .dotx (a .docm or .dotm kept so when the document is
//! one and the name says so) through the edit core's [`serialise`], cleaned when asked
//! ([`clean`]), or its Markdown (every story, the final view, no ids) -- given to the
//! application as the reply's file ([`ooxml_tools::File`]), never to the model; with
//! docx-agent's refusals of a deck's format, a kind the extension contradicts and `clean`
//! for Markdown. `app.xml`'s statistics are brought up to date only when the main part
//! changed since the document was opened: words, characters and paragraphs counted from
//! the body, pages and lines from the version's layout -- the host's counts where it gives
//! them ([`Version::counts`]), else the document's engine's -- and the reply's `coverage`
//! the engine's of the version saved (docx-agent's `_coverage`: a check that passed on a
//! complete layout told from one that could not see everything; `{"error"}` where it was
//! not laid out).
//!
//! Must not: run the save gate or keep the output's id state (the host's: it hands the
//! application the bytes), or change the document.

use crate::Version;
use ooxml_base::{Cancel, Limits};
use ooxml_tools::{ArgEnum, Done, File, Refusal, ToolArgs, ToolError, TryCall, WayOut};
use ooxml_word_edit::{clean, serialise, AppStatistics, FileKind, WordDoc};
use ooxml_word_model::docx::statistics::text_counts;
use ooxml_word_model::markdown::read::project;
use ooxml_word_model::markdown::render::render;
use ooxml_word_model::markdown::{Stories, ViewName};
use ooxml_word_model::parts::main_part;
use serde_json::{Map, Value};

/// The formats `save_document` names (every kind's).
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFormat {
    /// A deck.
    Pptx,
    /// A deck's template.
    Potx,
    /// A document.
    Docx,
    /// A document's template.
    Dotx,
    /// A workbook.
    Xlsx,
    /// A workbook with macros.
    Xlsm,
    /// A workbook's template.
    Xltx,
    /// A document's text copy.
    Markdown,
    /// A deck's text copy.
    Outline,
}

impl SaveFormat {
    fn name(self) -> &'static str {
        match self {
            Self::Pptx => "pptx",
            Self::Potx => "potx",
            Self::Docx => "docx",
            Self::Dotx => "dotx",
            Self::Xlsx => "xlsx",
            Self::Xlsm => "xlsm",
            Self::Xltx => "xltx",
            Self::Markdown => "markdown",
            Self::Outline => "outline",
        }
    }
}

/// `save_document`'s arguments, shared by every format.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "save_document",
    description = include_str!("save_document.md"),
    group = "core",
    role = "any",
    batchable = false
)]
pub struct SaveDocument {
    /// Document id.
    pub doc: String,
    /// File name, e.g. q3-review.pptx.
    pub name: String,
    /// potx/dotx/xltx: a template; outline (decks) or markdown (documents): a text copy.
    pub format: SaveFormat,
    /// Documents: strip what Word never draws (orphaned parts, rsids, web divs, template link). Default false.
    #[arg(kinds("docx"))]
    pub clean: Option<bool>,
}

/// The formats a document saves as.
const DOCUMENT_FORMATS: [&str; 3] = ["docx", "dotx", "markdown"];

impl SaveDocument {
    fn try_call(&self, format: &str, name: &str) -> TryCall {
        let mut arguments = Map::new();
        arguments.insert("doc".into(), Value::from(self.doc.as_str()));
        arguments.insert("name".into(), Value::from(name));
        arguments.insert("format".into(), Value::from(format));
        TryCall {
            tool: "save_document".to_owned(),
            arguments,
            fill: Vec::new(),
        }
    }

    /// The name with its extension `extension` in place of the one it has.
    fn renamed(&self, extension: &str) -> String {
        let stem = self.name.rsplit_once('.').map_or(self.name.as_str(), |(stem, _)| stem);
        format!("{stem}.{extension}")
    }
}

/// `save_document`: the file, its name and format, given to the application.
///
/// # Errors
/// docx-agent's refusals (`invalid_arguments` at the field; `refused` for macros a .docx
/// or .dotx cannot carry); `internal` where the package cannot be read or written.
pub fn save_document(
    args: &SaveDocument,
    version: &Version<'_>,
    cancel: &dyn Cancel,
) -> Result<Done<Value>, ToolError> {
    crate::review::stopped(cancel)?;
    refuse_format(args)?;
    if args.format == SaveFormat::Markdown {
        return markdown(args, version);
    }
    let format = args.format.name();
    let extension = args
        .name
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default();
    if ["docx", "dotx", "docm", "dotm"].contains(&extension.as_str()) && extension[..3] != format[..3] {
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            format!(
                "{} names a .{extension} but format is {format}: Word refuses a file whose kind and extension disagree",
                args.name
            ),
            WayOut::call(args.try_call(format, &args.renamed(format))),
        )
        .at("name"));
    }
    let Some(owned) = version.owned else {
        return Err(crate::refusal::not_supported(
            "save_document of a document the session does not own",
            "save_document",
            serde_json::json!({"doc": args.doc}),
            &["doc"],
        ));
    };
    let current = FileKind::of(version.package).map_err(internal)?;
    let mut kind = FileKind::named(format).unwrap_or(FileKind::Docx);
    if matches!(current, Some(FileKind::Docm | FileKind::Dotm)) {
        if let Some(named) = FileKind::named(&extension).filter(|k| k.macro_enabled()) {
            kind = named;
        }
    }
    let laid = crate::laid_reads::laid(&owned, version.package, cancel);
    let counts = version
        .counts
        .or_else(|| laid.as_ref().and_then(|l| l.as_ref().ok()).and_then(|l| l.counts()));
    let app = if main_changed(owned.opened, owned.doc)? {
        Some(statistics(version, counts)?)
    } else {
        None
    };
    let coverage = match laid {
        Some(Ok(laid)) => laid.coverage().unwrap_or_else(crate::laid_reads::error_json),
        Some(Err(said)) => crate::laid_reads::error_json(said),
        None => crate::laid_reads::error_json(NO_LAYOUT.to_owned()),
    };
    let data = match serialise(version.package, kind, app.as_ref()) {
        Ok(data) => data,
        Err(ooxml_word_edit::EditError::Refused(message)) => {
            return Err(ToolError::refusal(
                Refusal::Refused,
                message,
                WayOut::call(args.try_call("markdown", &args.renamed("md"))),
            ))
        }
        Err(e) => return Err(internal(e)),
    };
    file(args, data, coverage)
}

/// The coverage of a save the session lays out no pages for.
const NO_LAYOUT: &str = "layout_failed: the session lays this document out with no page engine";

fn internal(e: ooxml_word_edit::EditError) -> ToolError {
    ToolError::internal(e.to_string(), "word:save")
}

/// docx-agent's refusals of the format: a deck's, a workbook's, and `clean` for Markdown.
fn refuse_format(args: &SaveDocument) -> Result<(), ToolError> {
    let format = args.format.name();
    let formats = || WayOut::names(DOCUMENT_FORMATS).unwrap_or_else(|| WayOut::call(args.try_call("docx", &args.name)));
    if matches!(args.format, SaveFormat::Pptx | SaveFormat::Potx | SaveFormat::Outline) {
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            format!("{format} is a deck format; a document saves as docx, dotx or markdown"),
            formats(),
        )
        .at("format"));
    }
    if !DOCUMENT_FORMATS.contains(&format) {
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            format!("a document saves as docx, dotx or markdown, not {format}"),
            formats(),
        )
        .at("format"));
    }
    if args.format == SaveFormat::Markdown && args.clean.unwrap_or(false) {
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            "clean is for a docx or dotx; a Markdown copy carries none of it: leave clean out",
            WayOut::call(args.try_call("markdown", &args.name)),
        )
        .at("clean"));
    }
    Ok(())
}

/// The document's Markdown: every story, the final view, no ids.
fn markdown(args: &SaveDocument, version: &Version<'_>) -> Result<Done<Value>, ToolError> {
    let docx = version.docx()?;
    let blocks = project(&docx, None, ViewName::Final, Stories::All, false)
        .map_err(|e| ToolError::internal(e.to_string(), "word:markdown"))?;
    let mut done = Done::new(format!("Saved {} (Markdown) for the application", args.name));
    done.files.push(File {
        name: args.name.clone(),
        format: "markdown".to_owned(),
        bytes: render(&blocks).into_bytes(),
    });
    Ok(done)
}

/// `app.xml`'s statistics of the version: the body's text counted, the layout's pages and
/// lines (`counts`) where it was laid out.
fn statistics(version: &Version<'_>, counts: Option<crate::LayoutCounts>) -> Result<AppStatistics, ToolError> {
    let text = text_counts(&*version.docx()?);
    Ok(AppStatistics {
        words: text.words,
        characters: text.characters,
        characters_with_spaces: text.characters_with_spaces,
        paragraphs: text.paragraphs,
        pages: counts.map(|c| c.pages),
        lines: counts.map(|c| c.lines),
    })
}

/// The reply giving the application `data`, cleaned first when asked, with the layout's
/// `coverage`.
fn file(args: &SaveDocument, mut data: Vec<u8>, coverage: Value) -> Result<Done<Value>, ToolError> {
    let mut summary = format!("Saved {} for the application", args.name);
    let mut reply = Map::new();
    if args.clean.unwrap_or(false) {
        let (cleaned, report) = clean(&data, &Limits::default()).map_err(internal)?;
        data = cleaned;
        summary.push_str(&format!(
            ", cleaned: {} -> {} bytes",
            thousands(report.before),
            thousands(report.after)
        ));
        reply.insert("cleaned".into(), report.to_json());
    }
    reply.insert("coverage".into(), coverage);
    let mut done = Done::new(summary);
    done.data = Some(Value::Object(reply));
    done.files.push(File {
        name: args.name.clone(),
        format: args.format.name().to_owned(),
        bytes: data,
    });
    Ok(done)
}

/// Whether the main part of `now` is not the one the document was opened with.
fn main_changed(opened: &WordDoc, now: &WordDoc) -> Result<bool, ToolError> {
    let main = main_part(now.package()).map_err(|e| ToolError::internal(e.to_string(), "word:save"))?;
    Ok(
        main.is_some_and(|main| match (opened.package().part(&main), now.package().part(&main)) {
            (Some(before), Some(after)) => !before.same(after),
            _ => true,
        }),
    )
}

/// `n` with commas between thousands (Python's `f"{n:,}"`).
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Registers `save_document` (the group's [`super::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.read::<SaveDocument>(save_document);
}

#[cfg(test)]
mod tests {
    use super::thousands;

    #[test]
    fn thousands_as_python_writes_them() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(13_066), "13,066");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}

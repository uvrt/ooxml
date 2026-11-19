//! The style tools' cases of `word-edits` (`op: "style"`): `word_lists`, `word_format`,
//! `word_compare_blocks`, `word_follow_sample` (and the other style tools as they are ported) called with JSON arguments on a document as opened,
//! compared as the structure tools' cases are (`structure_tools::tool_case`), for
//! `python/word_edits_diff.py --style` to compare with docx-agent's toolbox.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::structure_tools::{run, tool_case};
use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_tools::ToolResult;
use ooxml_tools_word::{
    word_compare_blocks, word_follow_sample, word_format, word_lists, word_styles, word_template, Editing, Others,
    Version, WordCompareBlocks, WordFollowSample, WordFormat, WordLists, WordStyles, WordTemplate,
};
use ooxml_word_model::docx::Versions;
use serde_json::Value;
use std::sync::Arc;

struct Never;
impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// One style tool case on a freshly opened document.
///
/// # Errors
/// A message when the document cannot be opened.
pub fn case(bytes: &Arc<[u8]>, case: &Value) -> Result<Value, String> {
    tool_case(bytes, case, |tool, arguments, editing| match tool {
        "word_lists" => Some(run(arguments, |args: &WordLists| word_lists(args, editing))),
        "word_format" => Some(run(arguments, |args: &WordFormat| word_format(args, editing))),
        "word_styles" => Some(run(arguments, |args: &WordStyles| word_styles(args, editing))),
        "word_template" => Some(run(arguments, |args: &WordTemplate| word_template(args, editing))),
        "word_compare_blocks" => Some(run(arguments, |args: &WordCompareBlocks| {
            compare(bytes, case, editing.opened.package(), args)
        })),
        "word_follow_sample" => Some(run(arguments, |args: &WordFollowSample| {
            follow(bytes, case, editing, args)
        })),
        _ => None,
    })
}

/// `d2`, which a call may name: the document opened again (`reference_copy`) or the file
/// `reference_file`.
fn other_package(bytes: &Arc<[u8]>, case: &Value) -> Option<Package> {
    let other: Option<Arc<[u8]>> = match case.get("reference_file").and_then(Value::as_str) {
        Some(path) => std::fs::read(path).ok().map(Arc::from),
        None => case
            .get("reference_copy")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .then(|| Arc::clone(bytes)),
    };
    other.and_then(|b| Package::open(b, &Limits::default()).ok())
}

/// `word_follow_sample` on the document as opened, `d2` handed over as the session hands it.
fn follow(bytes: &Arc<[u8]>, case: &Value, editing: &Editing<'_>, args: &WordFollowSample) -> ToolResult<Value> {
    let versions = Versions::new();
    let copy = other_package(bytes, case);
    let others: Vec<(&str, Version<'_>)> = copy
        .iter()
        .map(|package| {
            let version = Version {
                name: "copy.docx",
                package,
                number: 1,
                versions: &versions,
                placer: None,
                others: Others::default(),
            };
            ("d2", version)
        })
        .collect();
    let editing = Editing {
        others: Others::new(&others),
        ..*editing
    };
    word_follow_sample(args, &editing)
}

/// `word_compare_blocks` on the document as opened; `d2`, which the call may name, is the
/// document opened again (`reference_copy`) or the file `reference_file`.
fn compare(bytes: &Arc<[u8]>, case: &Value, package: &Package, args: &WordCompareBlocks) -> ToolResult<Value> {
    let versions = (Versions::new(), Versions::new());
    let copy = other_package(bytes, case);
    let others: Vec<(&str, Version<'_>)> = copy
        .iter()
        .map(|package| {
            let version = Version {
                name: "copy.docx",
                package,
                number: 1,
                versions: &versions.1,
                placer: None,
                others: Others::default(),
            };
            ("d2", version)
        })
        .collect();
    let version = Version {
        name: "case.docx",
        package,
        number: 1,
        versions: &versions.0,
        placer: None,
        others: Others::new(&others),
    };
    match word_compare_blocks(args, &version, &Never) {
        Ok(done) => ToolResult::done(done),
        Err(error) => ToolResult::failed(error),
    }
}

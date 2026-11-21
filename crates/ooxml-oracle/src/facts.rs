//! `word-reads`' fact cases: `check`'s facts read from the document alone
//! (`ooxml_word_edit::facts`, worded by `ooxml_tools_word`), the problems in some blocks and
//! those only a save finds (`ooxml_word_edit::validate`), in the form
//! `python/word_facts_cases.py` writes docx-agent's (`layout_facts.py`'s, `validate.py`'s and
//! `tools/originals.py`'s functions): `{"facts": [...]}`, or `{"error"}`.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::Unlimited;
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_tools_word::{
    direct_formatting_json, empty_run_json, grid_mismatch_json, lost_cell_json, lost_original_json, row_unlike_json,
    skipped_level_json, style_use_json,
};
use ooxml_word_edit::validate::{extension_problems, problems, problems_in};
use ooxml_word_edit::{facts, EditError, WordDoc};
use ooxml_word_model::docx::at_path;
use ooxml_word_model::ids::{W, W14};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;

/// The body blocks a `problems_in` case reads alone at most
/// (`python/word_facts_cases.py`'s `BLOCKS`).
const BLOCKS: usize = 200;

/// The file names an `extension` case writes to (`python/word_facts_cases.py`'s `TARGETS`).
const TARGETS: [&str; 6] = ["out.docx", "out.DOCM", "out.dotx", "out.dotm", "out.txt", ".docx"];

/// The fact `fact` of `package`'s first version, uncapped; `opened`, where given, is the
/// file as the session opened it (`accept_loses_original`; else the package itself).
#[must_use]
pub fn reply(package: &Package, fact: &str, opened: Option<&str>) -> Value {
    let found = (|| -> Result<Option<Vec<Value>>, String> {
        let doc = WordDoc::open(package.clone(), Arc::new(Unlimited)).map_err(|e| e.to_string())?;
        let opened = match opened {
            Some(file) => open(file)?,
            None => doc.clone(),
        };
        entries(&doc, &opened, fact).map_err(|e| e.to_string())
    })();
    match found {
        Ok(Some(found)) => json!({"facts": found}),
        Ok(None) => json!({"error": format!("no fact {fact}")}),
        Err(error) => json!({"error": error}),
    }
}

/// The document in `file`, as the session opened it.
fn open(file: &str) -> Result<WordDoc, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
    let package = Package::open(Arc::from(bytes), &Limits::default()).map_err(|e| format!("{file}: {e}"))?;
    WordDoc::open(package, Arc::new(Unlimited)).map_err(|e| e.to_string())
}

fn entries(doc: &WordDoc, opened: &WordDoc, fact: &str) -> Result<Option<Vec<Value>>, EditError> {
    Ok(Some(match fact {
        "empty_paragraphs" => facts::empty_paragraphs(doc)?.iter().map(empty_run_json).collect(),
        "numbering_skips_level" => facts::numbering_skips_level(doc)?
            .iter()
            .map(skipped_level_json)
            .collect(),
        "table_grid_mismatch" => facts::table_grid_mismatch(doc, None)?
            .iter()
            .map(grid_mismatch_json)
            .collect(),
        "accept_loses_text" => facts::accept_loses_text(doc)?.iter().map(lost_cell_json).collect(),
        "direct_formatting" => facts::direct_formatting(doc)?
            .iter()
            .map(direct_formatting_json)
            .collect(),
        "styles" => {
            let found = facts::style_list(doc, &ALLOWED.map(str::to_owned))?;
            let entries: Vec<Value> = found.not_allowed.iter().map(style_use_json).collect();
            vec![json!({"styles_not_allowed": entries, "styles_unknown": found.unknown})]
        }
        "accept_loses_original" => facts::accept_loses_original(doc, opened.docx()?)?
            .iter()
            .map(lost_original_json)
            .collect(),
        "problems_in" => problems_in_blocks(doc)?,
        "extension" => TARGETS
            .iter()
            .map(|target| {
                let found = extension_problems(doc, target)?;
                let listed: Vec<String> = found.iter().map(ToString::to_string).collect();
                Ok(json!({"target": target, "problems": listed}))
            })
            .collect::<Result<_, EditError>>()?,
        "row_text_format" => facts::row_text_format(doc, &written(doc)?, None)?
            .iter()
            .map(row_unlike_json)
            .collect(),
        _ => return Ok(None),
    }))
}

/// The styles a `styles` case allows (`python/word_facts_cases.py`'s `ALLOWED`): a name in
/// another case, an id, a heading's name and one no style has.
const ALLOWED: [&str; 4] = ["normal", "TableGrid", "heading 1", "No Such Style"];

/// The problems in each of the body's first [`BLOCKS`] blocks alone (those holding one) and
/// in them all, with how many are elsewhere.
fn problems_in_blocks(doc: &WordDoc) -> Result<Vec<Value>, EditError> {
    let found = problems(doc)?;
    let docx = doc.docx()?;
    let Some(body) = docx.body().and_then(|story| story.root().find(Some(W), "body")) else {
        return Ok(Vec::new());
    };
    let blocks: Vec<_> = body.child_elements().take(BLOCKS).cloned().collect();
    let entry = |block: Value, roots: &[Arc<ooxml_xml::Element>]| {
        let scoped = problems_in(doc, &found, &docx.main, roots);
        let listed: Vec<String> = scoped.inside.iter().map(ToString::to_string).collect();
        (
            scoped.inside.is_empty(),
            json!({"block": block, "problems": listed, "elsewhere": scoped.elsewhere}),
        )
    };
    let mut out = Vec::new();
    for (k, block) in blocks.iter().enumerate() {
        let (empty, value) = entry(json!(k), std::slice::from_ref(block));
        if !empty {
            out.push(value);
        }
    }
    out.push(entry(json!("all"), &blocks).1);
    Ok(out)
}

/// The made-up rows written `row_text_format` is read with (`word_facts_cases.py`'s
/// `_written`): each body table's every third row from row 2, by `w14:paraId`.
fn written(doc: &WordDoc) -> Result<BTreeSet<String>, EditError> {
    let docx = doc.docx()?;
    let Some(body) = docx.body() else {
        return Ok(BTreeSet::new());
    };
    Ok(body
        .ids
        .tables()
        .iter()
        .flat_map(|t| t.rows.into_iter().skip(2).step_by(3))
        .filter_map(|(path, _)| {
            at_path(body.root(), &path)?
                .attribute(Some(W14), "paraId")
                .map(str::to_owned)
        })
        .collect())
}

//! Owns: what a host's tool layer reads of a Word version for a check of numbers and a
//! scale (lane H7e, the docx format's `number_texts` and `scale_values` hooks without the
//! Python view): the texts a check of numbers reads ([`ValuesOf::number_texts`]) and the values
//! a scale's targets hold ([`ValuesOf::scale_values`]), placed on their pages by the layout the
//! host gives (the session's own engine for the document, [`crate::Session::pages`]).
//!
//! Must not: change the document, or lay out but with the layout given.

use crate::WordVersion;
use ooxml_session::{CancelToken, Opened};
use ooxml_tools::ToolError;
use ooxml_tools_word::{NumberTexts, ReadScale, ScaleValues, WordLayout};

/// The numbers and scales a Word version's check reads.
pub trait ValuesOf {
    /// The texts a check of numbers reads in the version (`ooxml_tools_word::number_texts`):
    /// within `range` (a check's, refs resolved; refused in its words, `doc` named as the
    /// host names the document) and on `pages`, each placed by `layout`; `None` for a
    /// document that is not a Word document.
    ///
    /// # Errors
    /// The read's refusal; `internal` when the version cannot be read.
    fn number_texts(
        &self,
        doc: &str,
        range: Option<&[String]>,
        pages: Option<&[usize]>,
        layout: Option<&dyn WordLayout>,
    ) -> Result<Option<NumberTexts>, ToolError>;

    /// The values `scale`'s targets hold in the version (`ooxml_tools_word::scale_values`),
    /// within `range` and on `pages` as [`ValuesOf::number_texts`] reads them; `None` for a
    /// document that is not a Word document.
    ///
    /// # Errors
    /// The read's refusal; `internal` when the version cannot be read.
    fn scale_values(
        &self,
        doc: &str,
        scale: &ReadScale,
        range: Option<&[String]>,
        pages: Option<&[usize]>,
        layout: Option<&dyn WordLayout>,
    ) -> Result<Option<ScaleValues>, ToolError>;
}

impl ValuesOf for Opened {
    fn number_texts(
        &self,
        doc: &str,
        range: Option<&[String]>,
        pages: Option<&[usize]>,
        layout: Option<&dyn WordLayout>,
    ) -> Result<Option<NumberTexts>, ToolError> {
        let Some(word) = self.word() else {
            return Ok(None);
        };
        let docx = word
            .docx()
            .map_err(|e| ToolError::internal(e.to_string(), "number_texts:docx"))?;
        ooxml_tools_word::number_texts(word, &docx, doc, range, pages, layout, &CancelToken::new()).map(Some)
    }

    fn scale_values(
        &self,
        doc: &str,
        scale: &ReadScale,
        range: Option<&[String]>,
        pages: Option<&[usize]>,
        layout: Option<&dyn WordLayout>,
    ) -> Result<Option<ScaleValues>, ToolError> {
        let Some(word) = self.word() else {
            return Ok(None);
        };
        let docx = word
            .docx()
            .map_err(|e| ToolError::internal(e.to_string(), "scale_values:docx"))?;
        ooxml_tools_word::scale_values(word, &docx, doc, scale, range, pages, layout, &CancelToken::new()).map(Some)
    }
}

/// The texts a check of numbers reads in the Word version `version` of `word` (the docx
/// format's `number_texts` hook), within the check's scope `read` gives: its `range` (refs
/// resolved), its `pages` (or `slides` read as pages, saying so), each text placed on its
/// page by the call's layout, else the session's own engine for the document; a text's scope
/// is its address as a range (`{"range": at}`), its place its page.
///
/// # Errors
/// A slide that is no page number; the read's refusal (a range refused as `check` refuses
/// it, pages that cannot be laid out); `internal` when the version cannot be read.
pub(crate) fn number_texts(
    word: &crate::Word,
    version: &ooxml_store::Snapshot<ooxml_word_edit::WordDoc>,
    read: &ooxml_session::ValueRead<'_>,
) -> Result<ooxml_session::ReadTexts, ToolError> {
    let mut warnings = Vec::new();
    let pages = pages_of(read.named, read.scope, &mut warnings)?;
    let range = ranges_of(read.scope.get("range"));
    let none = crate::call::WordHost::default();
    let host = crate::call::WordHost::of(read.context).unwrap_or(&none);
    let layout = word.layout(host);
    let layout_cancel = read.cancel.layout();
    let cancel: &dyn ooxml_base::Cancel = match &layout_cancel {
        Some(limit) => limit,
        None => read.cancel,
    };
    let doc = version.doc();
    let docx = doc
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "number_texts:docx"))?;
    let range = (!range.is_empty()).then_some(range.as_slice());
    let found = ooxml_tools_word::number_texts(
        doc,
        &docx,
        read.named,
        range,
        pages.as_deref(),
        layout.as_deref(),
        cancel,
    )?;
    warnings.extend(found.warnings);
    let texts = found
        .texts
        .into_iter()
        .map(|text| {
            let mut scope = serde_json::Map::new();
            scope.insert("range".into(), serde_json::Value::from(text.at.as_str()));
            let mut place = serde_json::Map::new();
            if let Some(page) = text.page {
                place.insert("page".into(), serde_json::Value::from(page));
            }
            ooxml_session::numbers::NumberText {
                at: text.at,
                text: text.text,
                written: text.written,
                scope,
                place,
            }
        })
        .collect();
    Ok(ooxml_session::ReadTexts { texts, warnings })
}

/// A check's `range` as a list of addresses (docx-agent's `ranges_of`): one string is one
/// range; blanks are none; each once.
pub(crate) fn ranges_of(value: Option<&serde_json::Value>) -> Vec<String> {
    let items: Vec<&serde_json::Value> = match value {
        Some(serde_json::Value::Array(items)) => items.iter().collect(),
        Some(one @ serde_json::Value::String(_)) => vec![one],
        _ => Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for item in items {
        let Some(text) = item.as_str().map(ooxml_session::facts::py_strip) else {
            continue;
        };
        if !text.is_empty() && !out.iter().any(|o| o == text) {
            out.push(text.to_owned());
        }
    }
    out
}

/// A check's `pages`, or its `slides` read as pages (docx-agent's `pages_of`): given both,
/// `slides` is ignored; each said in `warnings`.
///
/// # Errors
/// `invalid_arguments` for a slide that is no page number.
pub(crate) fn pages_of(
    doc: &str,
    scope: &serde_json::Map<String, serde_json::Value>,
    warnings: &mut Vec<String>,
) -> Result<Option<Vec<usize>>, ToolError> {
    let pages = || -> Option<Vec<usize>> {
        scope.get("pages")?.as_array().map(|p| {
            p.iter()
                .filter_map(|n| n.as_u64().and_then(|n| usize::try_from(n).ok()))
                .collect()
        })
    };
    let Some(slides) = scope.get("slides").and_then(serde_json::Value::as_array) else {
        return Ok(pages());
    };
    if scope.get("pages").is_some() {
        warnings.push("slides ignored: a document has pages".to_owned());
        return Ok(pages());
    }
    warnings.push("slides read as pages: a document has pages".to_owned());
    let mut out = Vec::new();
    for (index, item) in slides.iter().enumerate() {
        let page = match item {
            serde_json::Value::String(text) => {
                let digits = ooxml_session::facts::py_strip(text);
                (ooxml_base::pystr::str_is_digit(digits))
                    .then(|| digits.parse::<usize>().ok())
                    .flatten()
                    .ok_or_else(|| slide_refused(doc, index, text))?
            }
            other => other
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| slide_refused(doc, index, &other.to_string()))?,
        };
        out.push(page);
    }
    Ok(Some(out))
}

/// A slide that is no page number refused, as `check` refuses it.
fn slide_refused(doc: &str, index: usize, given: &str) -> ToolError {
    let mut arguments = serde_json::Map::new();
    arguments.insert("doc".into(), serde_json::Value::from(doc));
    arguments.insert("pages".into(), serde_json::json!([1]));
    ToolError::refusal(
        ooxml_tools::Refusal::InvalidArguments,
        format!(
            "slides[{index}] is {}: a document has pages, numbered from 1 (pages: [1])",
            ooxml_base::pyfmt::repr_str(given)
        ),
        ooxml_tools::WayOut::call(ooxml_tools::TryCall {
            tool: "check".into(),
            arguments,
            fill: vec!["pages".into()],
        }),
    )
    .at("pages")
}

/// The values a scale's targets hold in the Word version `version` of `word` (the docx
/// format's `scale_values` hook), each with the step it is styled as now, within the check's
/// scope `read` gives; placed on their pages, by the call's layout else the document's own
/// engine, only where `pages` (or `slides`) are asked for.
///
/// # Errors
/// A slide that is no page number; the read's refusal; `internal` when the version cannot be
/// read.
pub(crate) fn scale_values(
    word: &crate::Word,
    version: &ooxml_store::Snapshot<ooxml_word_edit::WordDoc>,
    scale: &ooxml_session::scales::Scale,
    read: &ooxml_session::ValueRead<'_>,
) -> Result<ooxml_session::ReadValues, ToolError> {
    let mut warnings = Vec::new();
    let pages = pages_of(read.named, read.scope, &mut warnings)?;
    let range = ranges_of(read.scope.get("range"));
    let none = crate::call::WordHost::default();
    let host = crate::call::WordHost::of(read.context).unwrap_or(&none);
    let layout = pages.as_ref().and_then(|_| word.layout(host));
    let layout_cancel = read.cancel.layout();
    let cancel: &dyn ooxml_base::Cancel = match &layout_cancel {
        Some(limit) => limit,
        None => read.cancel,
    };
    let doc = version.doc();
    let docx = doc
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "scale_values:docx"))?;
    let asked = ReadScale {
        name: scale.name.clone(),
        steps: scale
            .steps
            .iter()
            .map(|s| ooxml_tools_word::ReadStep {
                fill: s.fill.clone(),
                text: s.text.clone(),
                label: s.label.clone(),
            })
            .collect(),
        targets: scale.targets.clone(),
        labels: scale.labels,
    };
    let range = (!range.is_empty()).then_some(range.as_slice());
    let found = ooxml_tools_word::scale_values(
        doc,
        &docx,
        read.named,
        &asked,
        range,
        pages.as_deref(),
        layout.as_deref(),
        cancel,
    )?;
    let values = found
        .values
        .into_iter()
        .map(|v| {
            let mut place = serde_json::Map::new();
            if let Some(page) = v.page {
                place.insert("page".into(), serde_json::Value::from(page));
            }
            ooxml_session::scales::ScaleValue {
                target: v.target,
                text: v.text,
                given: v.given,
                now: v.now,
                exact: v.exact,
                differs: v.differs,
                skip: v.skip,
                place,
                kept: v.kept.and_then(|k| k.as_object().cloned()),
                source: v.source,
            }
        })
        .collect();
    Ok(ooxml_session::ReadValues { values, warnings })
}

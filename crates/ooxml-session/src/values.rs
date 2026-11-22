//! Owns: what a host's tool layer reads of a Word version for a check of numbers and a
//! scale (lane H7e, the docx format's `number_texts` and `scale_values` hooks without the
//! Python view): the texts a check of numbers reads ([`Opened::number_texts`]) and the values
//! a scale's targets hold ([`Opened::scale_values`]), placed on their pages by the layout the
//! host gives (the session's own engine for the document, [`crate::Session::pages`]).
//!
//! Must not: change the document, or lay out but with the layout given.

use crate::{CancelToken, Opened};
use ooxml_tools::ToolError;
use ooxml_tools_word::{NumberTexts, ReadScale, ScaleValues, WordLayout};

impl Opened {
    /// The texts a check of numbers reads in the version (`ooxml_tools_word::number_texts`):
    /// within `range` (a check's, refs resolved; refused in its words, `doc` named as the
    /// host names the document) and on `pages`, each placed by `layout`; `None` for a
    /// document that is not a Word document.
    ///
    /// # Errors
    /// The read's refusal; `internal` when the version cannot be read.
    pub fn number_texts(
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

    /// The values `scale`'s targets hold in the version (`ooxml_tools_word::scale_values`),
    /// within `range` and on `pages` as [`Opened::number_texts`] reads them; `None` for a
    /// document that is not a Word document.
    ///
    /// # Errors
    /// The read's refusal; `internal` when the version cannot be read.
    pub fn scale_values(
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

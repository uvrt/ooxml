//! A form of a call the library does not support yet: one refusal, said one way
//! ([`ToolError::not_supported`]).
//!
//! Owns: the wording of such a refusal (`"<form> is not supported yet"`, then what the way
//! out offers), its code (`refused`, so the eleven codes pptx and xlsx share stay as they
//! are), the stable id of the form in `details.not_supported` (what the gates count and
//! allow), and the rule that it carries a way out (a call of a supported form, or the
//! options to choose from).
//!
//! Must not: know any format or tool; the tool crates name their forms.

use crate::error::{Refusal, ToolError, WayOut};
use crate::result::{Outcome, ToolResult};
use serde_json::Value;

/// The key of `details` that names the form not supported yet.
pub const NOT_SUPPORTED: &str = "not_supported";

impl ToolError {
    /// A refusal of a form the library does not support yet: `form` is the form's stable
    /// id (`"word_insert_markdown like"`), said in the message and kept in
    /// `details.not_supported`; `way_out` is what works instead.
    #[must_use]
    pub fn not_supported(form: &str, way_out: WayOut) -> Self {
        let instead = if way_out.try_call().is_some() {
            "try_call is a form that is supported"
        } else {
            "valid_options lists what is supported"
        };
        Self::refusal(
            Refusal::Refused,
            format!("{form} is not supported yet; {instead}"),
            way_out,
        )
        .with_detail(NOT_SUPPORTED, Value::from(form))
    }

    /// The form this error refuses as not supported yet, if it is such a refusal.
    #[must_use]
    pub fn not_supported_form(&self) -> Option<&str> {
        self.details().get(NOT_SUPPORTED).and_then(Value::as_str)
    }
}

impl<T> ToolResult<T> {
    /// The form the call was refused as not supported yet, if it was.
    #[must_use]
    pub fn not_supported_form(&self) -> Option<&str> {
        match &self.outcome {
            Outcome::Failed(error) => error.not_supported_form(),
            Outcome::Done(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::TryCall;

    #[test]
    fn says_the_form_and_keeps_its_id() {
        let call = TryCall {
            tool: "word_read".into(),
            arguments: serde_json::Map::new(),
            fill: Vec::new(),
        };
        let error = ToolError::not_supported("word_read final view", WayOut::call(call));
        let json = error.to_json();
        assert_eq!(json["code"], "refused");
        assert_eq!(
            json["message"],
            "word_read final view is not supported yet; try_call is a form that is supported"
        );
        assert_eq!(json["details"]["not_supported"], "word_read final view");
        assert_eq!(json["try_call"]["tool"], "word_read");
        assert_eq!(error.not_supported_form(), Some("word_read final view"));
        let other = ToolError::refusal(Refusal::Refused, "no", WayOut::one(Value::from("x")));
        assert_eq!(other.not_supported_form(), None);
    }
}

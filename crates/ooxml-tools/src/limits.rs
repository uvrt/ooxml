//! Owns: [`Limits`], the host's limits on what one call answers: its reply's characters, the
//! items a listing shows per page, the images it gives and their longest edge (docx-agent's
//! `Limits.max_result_chars`, `max_list_items`, `max_images_per_call`, `max_image_edge`);
//! and [`ImageBudget`], a budget of images the host keeps across calls (the session's, the
//! round's), with the refusal of images past the call's limit or a budget
//! ([`Limits::check_images`], docx-agent's `call.check_images`). A host gives them per call;
//! a tool reads them where it pages or draws.
//!
//! Must not: cut a reply ([`crate::ToolResult::to_text`] does), count the images a host
//! keeps (the host does), or know any tool.

use crate::{Failure, ToolError, MAX_RESULT_CHARS};
use serde_json::Value;

/// What a call's reply may hold, as the host sets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The reply within so many characters, as the model is sent it.
    pub result_chars: usize,
    /// The items a listing shows per page.
    pub list_items: usize,
    /// The images one call gives at most.
    pub images_per_call: usize,
    /// An image's longest edge, in pixels.
    pub image_edge: i64,
}

impl Limits {
    /// The defaults (24,000 characters, 50 items, 4 images of at most 2,576 pixels), as
    /// docx-agent's `Limits()`.
    pub const DEFAULT: Self = Self {
        result_chars: MAX_RESULT_CHARS,
        list_items: 50,
        images_per_call: 4,
        image_edge: 2576,
    };

    /// `count` images more refused (`limit`) where they pass the call's limit -- a call's
    /// images are its own tool's, none before it -- or one of `budgets`: the session's
    /// checked before the round's, each worded as docx-agent's `Session.check_images` words
    /// it, with its `budget`, `limit` and `remaining` as details.
    ///
    /// # Errors
    /// `limit` for the first limit `count` passes.
    pub fn check_images(&self, count: usize, budgets: &[ImageBudget]) -> Result<(), ToolError> {
        if count > self.images_per_call {
            return Err(ToolError::failure(
                Failure::Limit,
                format!("at most {} images per call", self.images_per_call),
            )
            .with_limit(self.images_per_call, count));
        }
        for span in [BudgetSpan::Session, BudgetSpan::Round] {
            for budget in budgets.iter().filter(|b| b.span == span) {
                if budget.used.saturating_add(count) > budget.limit {
                    return Err(budget.refusal());
                }
            }
        }
        Ok(())
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Which of the host's budgets of images an [`ImageBudget`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetSpan {
    /// The session's, across all its calls.
    Session,
    /// The round's: the calls of one model turn.
    Round,
}

impl BudgetSpan {
    /// Its name, as a host names it (`session`, `round`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Round => "round",
        }
    }

    /// The span a host names `name`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "session" => Some(Self::Session),
            "round" => Some(Self::Round),
            _ => None,
        }
    }
}

/// A budget of images the host keeps across calls: how many it allows, how many are used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageBudget {
    /// Whose budget.
    pub span: BudgetSpan,
    /// The images it allows.
    pub limit: usize,
    /// The images given so far.
    pub used: usize,
}

impl ImageBudget {
    /// The images it has room for still.
    #[must_use]
    pub fn left(&self) -> usize {
        self.limit.saturating_sub(self.used)
    }

    /// The refusal of images past it (`limit`, with its `budget`, `limit` and `remaining`).
    #[must_use]
    pub fn refusal(&self) -> ToolError {
        let message = match self.span {
            BudgetSpan::Session => format!(
                "the session's image budget is {} and {} are used; work from text (describe, check) from here",
                self.limit, self.used
            ),
            BudgetSpan::Round => format!(
                "this round's image budget is {} and {} are used; render again next round",
                self.limit, self.used
            ),
        };
        ToolError::failure(Failure::Limit, message)
            .with_detail("budget", Value::from(self.span.as_str()))
            .with_detail("limit", Value::from(self.limit))
            .with_detail("remaining", Value::from(self.left()))
    }
}

#[cfg(test)]
mod tests {
    use super::{BudgetSpan, ImageBudget, Limits};
    use serde_json::json;

    const fn budget(span: BudgetSpan, limit: usize, used: usize) -> ImageBudget {
        ImageBudget { span, limit, used }
    }

    #[test]
    fn images_past_the_call_or_a_budget_are_refused_as_docx_agent_words_it() {
        let limits = Limits::DEFAULT;
        let call = limits.check_images(5, &[]).unwrap_err();
        assert_eq!(call.message(), "at most 4 images per call");
        assert_eq!(
            serde_json::Value::Object(call.details().clone()),
            json!({"limit": 4, "given": 5})
        );
        let both = [budget(BudgetSpan::Round, 3, 3), budget(BudgetSpan::Session, 10, 9)];
        let session = limits.check_images(2, &both).unwrap_err();
        assert_eq!(
            session.message(),
            "the session's image budget is 10 and 9 are used; work from text (describe, check) from here"
        );
        assert_eq!(
            serde_json::Value::Object(session.details().clone()),
            json!({"budget": "session", "limit": 10, "remaining": 1})
        );
        let round = limits.check_images(1, &both).unwrap_err();
        assert_eq!(
            round.message(),
            "this round's image budget is 3 and 3 are used; render again next round"
        );
        assert!(limits.check_images(4, &[budget(BudgetSpan::Session, 4, 0)]).is_ok());
    }
}

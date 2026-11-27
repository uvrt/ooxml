//! Owns: what a session's tools produce for the application and what they may show the
//! model (design phase 3, section 5; ooxml-edit's `Session.add_output`/`take_outputs` and its
//! image budgets, ported exactly): files saved for the application, whose bytes never reach
//! the model ([`Output`], [`OutputStore`]: kept until taken, or handed straight to the
//! application's callback); and the images a session and a round may return
//! ([`ImageBudgets`]: counted as shown, given back when not sent, the round's started again
//! by the application).
//!
//! Must not: render an image or save a document (the formats do), or cut a reply.

use ooxml_tools::{BudgetSpan, ImageBudget, ToolError};
use serde_json::{Map, Value};
use std::any::Any;
use std::fmt;
use std::sync::Arc;

/// A file a tool produced for the application: its bytes never reach the model.
#[derive(Clone)]
pub struct Output {
    /// Its file name.
    pub name: String,
    /// Its format (`docx`, `pdf`).
    pub format: String,
    /// Its bytes.
    pub data: Arc<[u8]>,
    /// The document it is of.
    pub doc: Option<String>,
    /// What validating it found.
    pub validate: Option<Value>,
    /// The document's state beside these bytes, where the output is the document itself and
    /// its format keeps one: never described, the model does not see it.
    pub state: Option<Arc<dyn Any + Send + Sync>>,
}

impl fmt::Debug for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Output")
            .field("name", &self.name)
            .field("format", &self.format)
            .field("size", &self.data.len())
            .field("doc", &self.doc)
            .field("validate", &self.validate)
            .finish_non_exhaustive()
    }
}

impl Output {
    /// What a result says of it: never its bytes or its state.
    #[must_use]
    pub fn describe(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("name".into(), Value::from(self.name.as_str()));
        out.insert("format".into(), Value::from(self.format.as_str()));
        out.insert("size".into(), Value::from(self.data.len()));
        out.insert("doc".into(), Value::from(self.doc.as_deref()));
        out.insert("validate".into(), self.validate.clone().unwrap_or(Value::Null));
        out
    }
}

/// The application's callback for each output, in place of keeping it.
pub type OnOutput = Box<dyn Fn(Output) + Send + Sync>;

/// The outputs saved since the application last took them.
#[derive(Default)]
pub struct OutputStore {
    outputs: Vec<Output>,
    on_output: Option<OnOutput>,
}

impl fmt::Debug for OutputStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutputStore")
            .field("outputs", &self.outputs)
            .field("on_output", &self.on_output.is_some())
            .finish()
    }
}

impl OutputStore {
    /// Outputs kept until taken; or each handed to `on_output` at once.
    #[must_use]
    pub fn new(on_output: Option<OnOutput>) -> OutputStore {
        OutputStore {
            outputs: Vec::new(),
            on_output,
        }
    }

    /// Keep `output` for the application (or hand it over now).
    pub fn add(&mut self, output: Output) {
        match &self.on_output {
            Some(hand) => hand(output),
            None => self.outputs.push(output),
        }
    }

    /// The outputs saved since the last take, kept.
    #[must_use]
    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    /// The outputs saved since the last take, handed over and forgotten.
    pub fn take(&mut self) -> Vec<Output> {
        std::mem::take(&mut self.outputs)
    }
}

/// The images a session may return in all, and per round where the application starts
/// rounds: how many each allows (`None`: no cap) and how many are used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageBudgets {
    session: Option<usize>,
    per_round: Option<usize>,
    used: usize,
    this_round: usize,
    rounds: u64,
}

impl Default for ImageBudgets {
    /// 40 images a session (about 48,000 tokens of context at 1280x720), no per-round cap.
    fn default() -> Self {
        ImageBudgets::new(Some(40), None)
    }
}

impl ImageBudgets {
    /// Budgets of `session` images in all and `per_round` per round.
    #[must_use]
    pub fn new(session: Option<usize>, per_round: Option<usize>) -> ImageBudgets {
        ImageBudgets {
            session,
            per_round,
            used: 0,
            this_round: 0,
            rounds: 1,
        }
    }

    /// Start a round (per user message, say): the round's budget starts again; the
    /// session's does not.
    pub fn new_round(&mut self) {
        self.this_round = 0;
        self.rounds += 1;
    }

    /// The round the session is in, from 1.
    #[must_use]
    pub fn rounds(&self) -> u64 {
        self.rounds
    }

    /// The budgets as a call reads them: the session's, then the round's, where they cap.
    #[must_use]
    pub fn budgets(&self) -> Vec<ImageBudget> {
        let session = self.session.map(|limit| ImageBudget {
            span: BudgetSpan::Session,
            limit,
            used: self.used,
        });
        let round = self.per_round.map(|limit| ImageBudget {
            span: BudgetSpan::Round,
            limit,
            used: self.this_round,
        });
        session.into_iter().chain(round).collect()
    }

    /// Images still allowed now: the smaller of what the budgets leave; `None` when
    /// neither caps.
    #[must_use]
    pub fn remaining(&self) -> Option<usize> {
        self.budgets().iter().map(ImageBudget::left).min()
    }

    /// Refuse with `limit` when `count` images would pass the session's or the round's
    /// budget, counting none: a format asks before it renders, so a call that will be
    /// refused costs nothing.
    ///
    /// # Errors
    /// The first budget `count` passes (the session's before the round's).
    pub fn check(&self, count: usize) -> Result<(), ToolError> {
        match self.budgets().iter().find(|b| b.used.saturating_add(count) > b.limit) {
            Some(budget) => Err(budget.refusal()),
            None => Ok(()),
        }
    }

    /// Count `count` images against both budgets, or refuse with `limit`.
    ///
    /// # Errors
    /// As [`ImageBudgets::check`].
    pub fn reserve(&mut self, count: usize) -> Result<(), ToolError> {
        self.check(count)?;
        self.used += count;
        self.this_round += count;
        Ok(())
    }

    /// Give back `count` images reserved that the model is never sent (a `read_batch` op's
    /// that did not fit its reply): the budgets count what is shown.
    pub fn release(&mut self, count: usize) {
        self.used = self.used.saturating_sub(count);
        self.this_round = self.this_round.saturating_sub(count);
    }
}

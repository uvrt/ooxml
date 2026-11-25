//! What a tool reports when it does not do what was asked: [`ToolError`], its eleven codes,
//! and the way out a refusal must carry (tool design rule 2).
//!
//! Owns: the error codes and their meaning (as `results.py`'s `ERROR_CODES`), the error's
//! JSON (`code`, `message`, `field`, `valid_options`, `try_call`, `details`), and the rule
//! that a refusal -- `not_found`, `invalid_arguments`, `ambiguous`, `refused` -- cannot be
//! made without valid options or a call that would work: [`ToolError::refusal`] takes a
//! [`WayOut`], and a [`WayOut`] cannot be empty.
//!
//! Must not: know any format, provider or session; a library's own error enum maps to
//! these codes in the format's tool crate.

use serde_json::{Map, Value};

/// A refusal: the call cannot be done as asked, and the reply says what can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// An address, document or blob handle that does not exist.
    NotFound,
    /// Arguments that do not match the tool's schema, or an unknown tool.
    InvalidArguments,
    /// A target that matches more than once.
    Ambiguous,
    /// An edit the library refuses, or a layout that cannot fit.
    Refused,
}

/// A failure that is not a refusal: a way out is welcome but not required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// A label that names nothing.
    LabelNotFound,
    /// A length or value in the wrong unit or out of range.
    Unit,
    /// An undo whose change later changes depend on.
    Entangled,
    /// The deadline passed.
    Timeout,
    /// Laying the document out failed.
    LayoutFailed,
    /// A size or budget limit.
    Limit,
    /// An unexpected failure in the tool layer or a library.
    Internal,
}

/// Every error code: a [`Refusal`] or a [`Failure`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// A refusal.
    Refusal(Refusal),
    /// Another failure.
    Failure(Failure),
}

impl ErrorCode {
    /// All eleven, in `ERROR_CODES`' order.
    pub const ALL: [ErrorCode; 11] = [
        Self::Refusal(Refusal::NotFound),
        Self::Failure(Failure::LabelNotFound),
        Self::Refusal(Refusal::Ambiguous),
        Self::Refusal(Refusal::Refused),
        Self::Failure(Failure::Unit),
        Self::Refusal(Refusal::InvalidArguments),
        Self::Failure(Failure::Entangled),
        Self::Failure(Failure::Timeout),
        Self::Failure(Failure::LayoutFailed),
        Self::Failure(Failure::Limit),
        Self::Failure(Failure::Internal),
    ];

    /// The code as the model reads it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Refusal(Refusal::NotFound) => "not_found",
            Self::Refusal(Refusal::InvalidArguments) => "invalid_arguments",
            Self::Refusal(Refusal::Ambiguous) => "ambiguous",
            Self::Refusal(Refusal::Refused) => "refused",
            Self::Failure(Failure::LabelNotFound) => "label_not_found",
            Self::Failure(Failure::Unit) => "unit",
            Self::Failure(Failure::Entangled) => "entangled",
            Self::Failure(Failure::Timeout) => "timeout",
            Self::Failure(Failure::LayoutFailed) => "layout_failed",
            Self::Failure(Failure::Limit) => "limit",
            Self::Failure(Failure::Internal) => "internal",
        }
    }

    /// Whether an error of this code must carry a way out.
    #[must_use]
    pub fn needs_way_out(self) -> bool {
        matches!(self, Self::Refusal(_))
    }
}

/// A call that would work: the tool and its arguments. `fill` names the argument paths
/// whose value is only an example of the kind wanted, for the caller to replace.
#[derive(Debug, Clone, PartialEq)]
pub struct TryCall {
    /// The tool to call.
    pub tool: String,
    /// Its arguments.
    pub arguments: Map<String, Value>,
    /// The argument paths (`items[2].text`) whose value is an example to replace.
    pub fill: Vec<String>,
}

impl TryCall {
    /// The call as JSON: `{tool, arguments, fill}` (`fill` only when there is one).
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("tool".into(), Value::from(self.tool.as_str()));
        out.insert("arguments".into(), Value::Object(self.arguments.clone()));
        if !self.fill.is_empty() {
            out.insert(
                "fill".into(),
                self.fill.iter().map(|f| Value::from(f.as_str())).collect(),
            );
        }
        Value::Object(out)
    }
}

/// The way out of a refusal: valid options, a call that would work, or both. Never empty.
#[derive(Debug, Clone, PartialEq)]
pub struct WayOut {
    options: Vec<Value>,
    call: Option<TryCall>,
}

impl WayOut {
    /// The options to choose from; `None` when there are none (find another way out).
    #[must_use]
    pub fn options(options: Vec<Value>) -> Option<Self> {
        (!options.is_empty()).then_some(Self { options, call: None })
    }

    /// The options, as strings.
    #[must_use]
    pub fn names<I: IntoIterator<Item = S>, S: Into<String>>(names: I) -> Option<Self> {
        Self::options(names.into_iter().map(|n| Value::String(n.into())).collect())
    }

    /// One option.
    #[must_use]
    pub fn one(option: Value) -> Self {
        Self {
            options: vec![option],
            call: None,
        }
    }

    /// A call that would work.
    #[must_use]
    pub fn call(call: TryCall) -> Self {
        Self {
            options: Vec::new(),
            call: Some(call),
        }
    }

    /// This way out with a call that would work as well.
    #[must_use]
    pub fn and_call(mut self, call: TryCall) -> Self {
        self.call = Some(call);
        self
    }

    /// The valid options (possibly none, when there is a call).
    #[must_use]
    pub fn valid_options(&self) -> &[Value] {
        &self.options
    }

    /// The call that would work, if there is one.
    #[must_use]
    pub fn try_call(&self) -> Option<&TryCall> {
        self.call.as_ref()
    }
}

/// An error a tool reports to the model (boxed: it travels in every `Result` of the
/// contract, and the success path stays small).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{}: {}", .0.code.as_str(), .0.message)]
pub struct ToolError(Box<Body>);

#[derive(Debug, Clone, PartialEq)]
struct Body {
    code: ErrorCode,
    message: String,
    field: Option<String>,
    way_out: Option<WayOut>,
    details: Map<String, Value>,
}

impl ToolError {
    /// A refusal, with its way out.
    #[must_use]
    pub fn refusal(code: Refusal, message: impl Into<String>, way_out: WayOut) -> Self {
        Self::make(ErrorCode::Refusal(code), message.into(), Some(way_out))
    }

    /// A failure that is not a refusal.
    #[must_use]
    pub fn failure(code: Failure, message: impl Into<String>) -> Self {
        Self::make(ErrorCode::Failure(code), message.into(), None)
    }

    /// An unexpected failure, with the trace id the application's log has it under.
    #[must_use]
    pub fn internal(message: impl Into<String>, trace: &str) -> Self {
        Self::failure(Failure::Internal, message).with_detail("trace", Value::from(trace))
    }

    fn make(code: ErrorCode, message: String, way_out: Option<WayOut>) -> Self {
        Self(Box::new(Body {
            code,
            message,
            field: None,
            way_out,
            details: Map::new(),
        }))
    }

    /// The error naming the argument it is about (`box.x`, `cells[2].text`).
    #[must_use]
    pub fn at(mut self, field: impl Into<String>) -> Self {
        let field = field.into();
        self.0.field = (!field.is_empty()).then_some(field);
        self
    }

    /// The error with a way out added (a failure's is optional; a refusal's is replaced).
    #[must_use]
    pub fn with_way_out(mut self, way_out: WayOut) -> Self {
        self.0.way_out = Some(way_out);
        self
    }

    /// The error said in other words (a host's, where it knows more than the tool: the
    /// action a name meant), its code, field, way out and details kept.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.0.message = message.into();
        self
    }

    /// The error with one more detail.
    #[must_use]
    pub fn with_detail(mut self, key: &str, value: Value) -> Self {
        self.0.details.insert(key.into(), value);
        self
    }

    /// The code.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.0.code
    }

    /// The message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.0.message
    }

    /// The argument it is about.
    #[must_use]
    pub fn field(&self) -> Option<&str> {
        self.0.field.as_deref()
    }

    /// The way out, if any (always, for a refusal).
    #[must_use]
    pub fn way_out(&self) -> Option<&WayOut> {
        self.0.way_out.as_ref()
    }

    /// The details.
    #[must_use]
    pub fn details(&self) -> &Map<String, Value> {
        &self.0.details
    }

    /// The error as the model reads it: `code`, `message`, `field` (when there is one),
    /// `valid_options` (always, as today), `try_call` (when there is one), `details`
    /// (when there are some).
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("code".into(), Value::from(self.0.code.as_str()));
        out.insert("message".into(), Value::from(self.0.message.as_str()));
        if let Some(field) = &self.0.field {
            out.insert("field".into(), Value::from(field.as_str()));
        }
        let options = self.0.way_out.as_ref().map_or(&[][..], WayOut::valid_options);
        out.insert("valid_options".into(), Value::Array(options.to_vec()));
        if let Some(call) = self.0.way_out.as_ref().and_then(WayOut::try_call) {
            out.insert("try_call".into(), call.to_json());
        }
        if !self.0.details.is_empty() {
            out.insert("details".into(), Value::Object(self.0.details.clone()));
        }
        Value::Object(out)
    }
}

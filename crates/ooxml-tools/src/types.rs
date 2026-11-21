//! The Rust types an argument may have, and the schema node each stands for.
//!
//! Owns: [`ArgType`] (a type's schema node and how it is read from validated JSON) for
//! strings, whole numbers, numbers, booleans, `Option` (an optional argument, or a null
//! item: `orNull`), `Vec` (an array), a free-form object; [`IntOrStr`] (`orInteger`) and
//! [`NumOrStr`] (`orNumber`); [`ArgEnum`] for the enums `#[derive(ArgEnum)]` makes; and
//! [`ToolArgs`], a tool's arguments with its [`ToolSpec`].
//!
//! Must not: validate. Reading runs after [`crate::validate`], so a value of the wrong
//! shape here is a fault of the tool contract ([`ArgError`]), never the model's.

use crate::error::ToolError;
use crate::intent::Accepted;
use crate::schema::{Kind, NumberNode, Schema, StringNode};
use crate::spec::ToolSpec;
use serde_json::{Map, Value};

/// A validated value that does not have its type's shape: a fault in the contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the validated argument {at} is not {wanted}")]
pub struct ArgError {
    /// Where.
    pub at: String,
    /// What was wanted.
    pub wanted: &'static str,
}

impl ArgError {
    /// A value at `at` that is not `wanted`.
    #[must_use]
    pub fn new(at: &str, wanted: &'static str) -> Self {
        Self {
            at: if at.is_empty() { "arguments".into() } else { at.into() },
            wanted,
        }
    }
}

/// A type an argument may have.
pub trait ArgType: Sized {
    /// Whether an argument of this type may be left out (`Option`).
    const OPTIONAL: bool = false;

    /// The type's schema node, with no description.
    fn schema() -> Schema;

    /// The node as an array's item (`Option<T>`: `T`'s, taking `null`).
    #[must_use]
    fn item_schema() -> Schema {
        Self::schema()
    }

    /// The value read from validated JSON.
    ///
    /// # Errors
    /// [`ArgError`] when the value does not have the type's shape.
    fn from_arg(value: &Value, at: &str) -> Result<Self, ArgError>;

    /// The problems of attributes that do not fit their fields, in this type and the
    /// types it holds (a derived struct's; none for the others).
    #[must_use]
    fn definition_problems() -> Vec<String> {
        Vec::new()
    }

    /// The value of an argument left out.
    ///
    /// # Errors
    /// [`ArgError`] for a required type.
    fn absent(at: &str) -> Result<Self, ArgError> {
        Err(ArgError::new(at, "given"))
    }
}

/// The arguments of an editing tool some of whose calls only read (today's `Tool.reads`,
/// `edit_chart`'s `action: "read"`): such a call reads the version it pins, changing
/// nothing.
pub trait ReadsWhen {
    /// Whether this call only reads.
    fn reads(&self) -> bool;
}

/// A tool's arguments: one struct, from which `#[derive(ToolArgs)]` makes the spec.
pub trait ToolArgs: ArgType {
    /// The tool's definition.
    fn spec() -> ToolSpec;

    /// Every way the definition is wrong: the spec's own checks and the attributes that do
    /// not fit their fields. A test asserts it is empty for every tool.
    #[must_use]
    fn problems() -> Vec<String> {
        let mut problems = Self::spec().problems();
        problems.extend(Self::definition_problems());
        problems
    }

    /// A call's arguments taken as meant, validated and read, for a document of `kind`
    /// (`None`: a call that names none): the arguments, and the notes on what was read as
    /// what (the walk's first, then the kind rules').
    ///
    /// # Errors
    /// `invalid_arguments` with its way out; `internal` if the validated arguments do not
    /// read as the struct (a fault in the contract).
    fn accept(arguments: Value, kind: Option<&str>) -> Result<(Self, Vec<String>), ToolError> {
        let spec = Self::spec();
        let Accepted {
            mut arguments,
            mut notes,
        } = spec.accept(arguments)?;
        notes.extend(spec.ignored_for(kind, &mut arguments));
        let read = Self::from_arg(&Value::Object(arguments), "")
            .map_err(|e| ToolError::internal(e.to_string(), &format!("{}:arguments", spec.name)))?;
        Ok((read, notes))
    }
}

/// A unit enum of string values, from `#[derive(ArgEnum)]`.
pub trait ArgEnum: Sized + Copy {
    /// The values, in declaration order.
    const VALUES: &'static [&'static str];

    /// The value as written.
    fn as_str(self) -> &'static str;

    /// The variant written `text`.
    fn from_str_value(text: &str) -> Option<Self>;
}

impl ArgType for String {
    fn schema() -> Schema {
        Schema::string()
    }

    fn from_arg(value: &Value, at: &str) -> Result<Self, ArgError> {
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| ArgError::new(at, "a string"))
    }
}

impl ArgType for bool {
    fn schema() -> Schema {
        Schema::new(Kind::Boolean)
    }

    fn from_arg(value: &Value, at: &str) -> Result<Self, ArgError> {
        value.as_bool().ok_or_else(|| ArgError::new(at, "a boolean"))
    }
}

impl ArgType for i64 {
    fn schema() -> Schema {
        Schema::new(Kind::Integer(NumberNode::default()))
    }

    fn from_arg(value: &Value, at: &str) -> Result<Self, ArgError> {
        value.as_i64().ok_or_else(|| ArgError::new(at, "a whole number"))
    }
}

impl ArgType for f64 {
    fn schema() -> Schema {
        Schema::new(Kind::Number(NumberNode::default()))
    }

    fn from_arg(value: &Value, at: &str) -> Result<Self, ArgError> {
        value.as_f64().ok_or_else(|| ArgError::new(at, "a number"))
    }
}

/// A free-form object, for a tool never sent strict: the dispatcher checks its content
/// against another schema.
impl ArgType for Map<String, Value> {
    fn schema() -> Schema {
        Schema::new(Kind::Free)
    }

    fn from_arg(value: &Value, at: &str) -> Result<Self, ArgError> {
        value.as_object().cloned().ok_or_else(|| ArgError::new(at, "an object"))
    }
}

impl<T: ArgType> ArgType for Option<T> {
    const OPTIONAL: bool = true;

    fn schema() -> Schema {
        T::schema()
    }

    fn definition_problems() -> Vec<String> {
        T::definition_problems()
    }

    fn item_schema() -> Schema {
        let mut schema = T::item_schema();
        schema.or_null = true;
        schema
    }

    fn from_arg(value: &Value, at: &str) -> Result<Self, ArgError> {
        if value.is_null() {
            return Ok(None);
        }
        T::from_arg(value, at).map(Some)
    }

    fn absent(_at: &str) -> Result<Self, ArgError> {
        Ok(None)
    }
}

impl<T: ArgType> ArgType for Vec<T> {
    fn schema() -> Schema {
        Schema::array(T::item_schema())
    }

    fn definition_problems() -> Vec<String> {
        T::definition_problems()
    }

    fn from_arg(value: &Value, at: &str) -> Result<Self, ArgError> {
        let items = value.as_array().ok_or_else(|| ArgError::new(at, "an array"))?;
        items
            .iter()
            .enumerate()
            .map(|(i, item)| T::from_arg(item, &format!("{at}[{i}]")))
            .collect()
    }
}

/// A string that may also be given as a whole number of at least `MIN`, kept a number
/// (`orInteger`): an address that may be a position (`2`, or `"s:256"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntOrStr<const MIN: i64> {
    /// Given as a number.
    Int(i64),
    /// Given as a string.
    Str(String),
}

impl<const MIN: i64> ArgType for IntOrStr<MIN> {
    fn schema() -> Schema {
        Schema::new(Kind::String(StringNode {
            or_integer: Some(MIN),
            ..StringNode::default()
        }))
    }

    fn from_arg(value: &Value, at: &str) -> Result<Self, ArgError> {
        match value {
            Value::String(text) => Ok(Self::Str(text.clone())),
            Value::Number(n) => n
                .as_i64()
                .map(Self::Int)
                .ok_or_else(|| ArgError::new(at, "a whole number")),
            _ => Err(ArgError::new(at, "a string or a whole number")),
        }
    }
}

/// A string that may also be given as any finite number, kept a number (`orNumber`): a
/// value that is text or a number (`3` or `"3.5"`).
#[derive(Debug, Clone, PartialEq)]
pub enum NumOrStr {
    /// Given as a number.
    Num(f64),
    /// Given as a string.
    Str(String),
}

impl ArgType for NumOrStr {
    fn schema() -> Schema {
        Schema::new(Kind::String(StringNode {
            or_number: true,
            ..StringNode::default()
        }))
    }

    fn from_arg(value: &Value, at: &str) -> Result<Self, ArgError> {
        match value {
            Value::String(text) => Ok(Self::Str(text.clone())),
            Value::Number(n) => n.as_f64().map(Self::Num).ok_or_else(|| ArgError::new(at, "a number")),
            _ => Err(ArgError::new(at, "a string or a number")),
        }
    }
}

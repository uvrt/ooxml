//! The attributes the derives read: `#[tool(...)]` on a tool's struct, `#[arg(...)]` on a
//! field or an enum variant, and doc comments as descriptions.
//!
//! Owns: parsing them into plain values ([`ToolAttrs`], [`FieldAttrs`]), and the errors of
//! an unknown or malformed attribute, pointing at it.
//!
//! Must not: generate code (`expand` does).

use syn::meta::ParseNestedMeta;
use syn::parse::Parse;
use syn::{Attribute, Expr, ExprLit, ExprUnary, Lit, LitBool, LitStr, Token, UnOp};

/// `#[tool(...)]`.
#[derive(Default)]
pub struct ToolAttrs {
    pub name: Option<LitStr>,
    pub description: Option<Expr>,
    pub group: Option<LitStr>,
    pub flags: Vec<(String, bool)>,
    pub role: Option<LitStr>,
    pub priority: Option<Expr>,
    pub route: Option<LitStr>,
    pub documents: Option<Vec<LitStr>>,
    pub exactly_one: Vec<Vec<LitStr>>,
}

/// One modifier of a field's property, in the order written: a call on `ooxml_tools::support::Field`.
pub enum Modifier {
    /// A call with no argument (`single_as_list`, `refs`).
    Flag(String),
    /// A numeric bound (`minimum`, `maximum`): the literal and whether it is a float.
    Bound(String, Expr, bool),
    /// A count or length (`min_length`, `max_items`).
    Count(String, Expr),
    /// One string (`alias`).
    Str(String, LitStr),
    /// Two strings (`value_alias`, `kind_description`).
    Pair(String, LitStr, LitStr),
    /// A string and a list of strings (`kind_value`).
    Kinds(String, Option<LitStr>, Vec<LitStr>),
    /// A JSON literal (`default`, `kind_neutral`).
    Value(String, Expr),
    /// `format`.
    Format(LitStr),
}

/// `#[arg(...)]` on a field, and its doc comment.
#[derive(Default)]
pub struct FieldAttrs {
    pub rename: Option<LitStr>,
    pub description: Option<String>,
    pub modifiers: Vec<Modifier>,
}

fn flag(meta: &ParseNestedMeta) -> syn::Result<bool> {
    if meta.input.peek(Token![=]) {
        let value: LitBool = meta.value()?.parse()?;
        return Ok(value.value);
    }
    Ok(true)
}

/// The string literals in `name("a", "b")`.
fn string_list(meta: &ParseNestedMeta) -> syn::Result<Vec<LitStr>> {
    let content;
    syn::parenthesized!(content in meta.input);
    let list = content.parse_terminated(|input| input.parse::<LitStr>(), Token![,])?;
    Ok(list.into_iter().collect())
}

/// The literals in `name(1, "a", true)`.
fn literal_list(meta: &ParseNestedMeta) -> syn::Result<Vec<Expr>> {
    let content;
    syn::parenthesized!(content in meta.input);
    let list = content.parse_terminated(Expr::parse, Token![,])?;
    Ok(list.into_iter().collect())
}

/// Whether `expr` is a float literal, or a whole number one (both possibly negated);
/// `None` when it is not a number.
pub fn number_kind(expr: &Expr) -> Option<bool> {
    match expr {
        Expr::Lit(ExprLit { lit: Lit::Int(_), .. }) => Some(false),
        Expr::Lit(ExprLit { lit: Lit::Float(_), .. }) => Some(true),
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_), expr, ..
        }) => number_kind(expr),
        _ => None,
    }
}

/// Reads `#[tool(...)]` from `attrs`.
pub fn tool_attrs(attrs: &[Attribute]) -> syn::Result<Option<ToolAttrs>> {
    let Some(attr) = attrs.iter().find(|a| a.path().is_ident("tool")) else {
        return Ok(None);
    };
    let mut out = ToolAttrs::default();
    attr.parse_nested_meta(|meta| {
        let key = meta.path.get_ident().map(ToString::to_string).unwrap_or_default();
        match key.as_str() {
            "name" => out.name = Some(meta.value()?.parse()?),
            "description" => out.description = Some(meta.value()?.parse()?),
            "group" => out.group = Some(meta.value()?.parse()?),
            "role" => out.role = Some(meta.value()?.parse()?),
            "route" => out.route = Some(meta.value()?.parse()?),
            "priority" => out.priority = Some(meta.value()?.parse()?),
            "documents" => out.documents = Some(string_list(&meta)?),
            "exactly_one" => out.exactly_one.push(string_list(&meta)?),
            "mutates" | "strict" | "batchable" | "read_batchable" | "retry_key" => {
                let value = flag(&meta)?;
                out.flags.push((key, value));
            }
            _ => return Err(meta.error(format!("unknown tool attribute `{key}`"))),
        }
        Ok(())
    })?;
    Ok(Some(out))
}

/// The doc comment of `attrs` as one description: its lines trimmed and joined with
/// spaces; `None` when there is none.
pub fn doc_text(attrs: &[Attribute]) -> Option<String> {
    let mut lines = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("doc")) {
        if let syn::Meta::NameValue(pair) = &attr.meta {
            if let Expr::Lit(ExprLit {
                lit: Lit::Str(text), ..
            }) = &pair.value
            {
                let line = text.value().trim().to_owned();
                if !line.is_empty() {
                    lines.push(line);
                }
            }
        }
    }
    (!lines.is_empty()).then(|| lines.join(" "))
}

/// Reads a field's (or a variant's) `#[arg(...)]` and doc comment.
pub fn field_attrs(attrs: &[Attribute]) -> syn::Result<FieldAttrs> {
    let mut out = FieldAttrs {
        description: doc_text(attrs),
        ..FieldAttrs::default()
    };
    for attr in attrs.iter().filter(|a| a.path().is_ident("arg")) {
        attr.parse_nested_meta(|meta| field_attr(&meta, &mut out))?;
    }
    Ok(out)
}

fn field_attr(meta: &ParseNestedMeta, out: &mut FieldAttrs) -> syn::Result<()> {
    let key = meta.path.get_ident().map(ToString::to_string).unwrap_or_default();
    let modifier = match key.as_str() {
        "rename" => {
            out.rename = Some(meta.value()?.parse()?);
            return Ok(());
        }
        "description" => {
            let text: LitStr = meta.value()?.parse()?;
            out.description = Some(text.value());
            return Ok(());
        }
        "single_as_list" | "map_as_pairs" | "empty_is_absent" | "refs" | "datasets" | "sources" | "facts" => {
            Modifier::Flag(key)
        }
        "minimum" | "maximum" => {
            let value: Expr = meta.value()?.parse()?;
            let Some(float) = number_kind(&value) else {
                return Err(meta.error("expected a number literal"));
            };
            Modifier::Bound(key, value, float)
        }
        "min_length" | "max_length" | "min_items" | "max_items" => Modifier::Count(key, meta.value()?.parse()?),
        "alias" => Modifier::Str(key, meta.value()?.parse()?),
        "format" => Modifier::Format(meta.value()?.parse()?),
        "default" => Modifier::Value(key, meta.value()?.parse()?),
        "value_alias" | "kind_description" => match string_list(meta)?.as_slice() {
            [a, b] => Modifier::Pair(key, a.clone(), b.clone()),
            _ => return Err(meta.error(format!("`{key}` takes two strings"))),
        },
        "kinds" => Modifier::Kinds(key, None, string_list(meta)?),
        "kind_value" => {
            let mut list = string_list(meta)?.into_iter();
            let Some(value) = list.next() else {
                return Err(meta.error("`kind_value` takes a value and its kinds"));
            };
            Modifier::Kinds(key, Some(value), list.collect())
        }
        "kind_neutral" => {
            for value in literal_list(meta)? {
                out.modifiers.push(Modifier::Value(key.clone(), value));
            }
            return Ok(());
        }
        _ => return Err(meta.error(format!("unknown arg attribute `{key}`"))),
    };
    out.modifiers.push(modifier);
    Ok(())
}

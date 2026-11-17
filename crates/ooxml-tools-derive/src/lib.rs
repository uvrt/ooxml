//! `#[derive(ToolArgs)]` and `#[derive(ArgEnum)]` for `ooxml-tools` (docs/architecture/
//! README.md, section 9; layer 4).
//!
//! Owns: reading a struct's `#[tool(...)]` and its fields' `#[arg(...)]` and doc comments,
//! and an enum's variants, and writing the `ooxml_tools` trait impls that build the tool's
//! schema, intent rules and spec from them. Every rule lives in `ooxml_tools::support`; the
//! generated code only names the attributes in order.
//!
//! Must not: decide anything the runtime crate decides (schema form, validation, messages);
//! a wrong attribute for a field's type is reported by `ToolArgs::problems`, which a test
//! asserts empty for every tool.
#![forbid(unsafe_code)]

mod attrs;
mod expand;

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

/// A tool's arguments, or an object inside them: the schema of each field from its type,
/// its description from its doc comment, its bounds and intent rules from `#[arg(...)]`;
/// with `#[tool(name = ..., description = ...)]` on the struct, the tool's spec too.
#[proc_macro_derive(ToolArgs, attributes(tool, arg))]
pub fn derive_tool_args(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand::tool_args(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// A string argument of fixed values: one per unit variant, written in snake case or as
/// `#[arg(rename = "...")]` says.
#[proc_macro_derive(ArgEnum, attributes(arg))]
pub fn derive_arg_enum(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand::arg_enum(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

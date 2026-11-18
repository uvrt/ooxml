//! The code the derives write: `ooxml_tools::ArgType` (and `ToolArgs`) for a struct,
//! `ooxml_tools::ArgEnum` and `ArgType` for an enum.
//!
//! Owns: turning parsed attributes into calls on `ooxml_tools::support::Field`, in the
//! order written, and the tool spec's settings.
//!
//! Must not: hold any rule of the contract; it only names them.

use crate::attrs::{field_attrs, number_kind, tool_attrs, Modifier, ToolAttrs};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, LitStr};

/// `ArgType` (and with `#[tool]`, `ToolArgs`) for a struct of named fields.
pub fn tool_args(input: &DeriveInput) -> syn::Result<TokenStream> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "ToolArgs needs a struct; use ArgEnum for an enum",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(&input.ident, "ToolArgs needs named fields"));
    };
    let ident = &input.ident;
    let mut made = Vec::new();
    let mut readers = Vec::new();
    let mut nested = Vec::new();
    for field in &fields.named {
        let Some(name) = &field.ident else { continue };
        let ty = &field.ty;
        let attrs = field_attrs(&field.attrs)?;
        let written = attrs.rename.as_ref().map_or_else(|| name.to_string(), LitStr::value);
        let mut chain = quote! { ::ooxml_tools::support::Field::of::<#ty>(#written) };
        if let Some(text) = &attrs.description {
            chain = quote! { #chain.describe(#text) };
        }
        for modifier in &attrs.modifiers {
            chain = modify(chain, modifier)?;
        }
        made.push(quote! { #chain.build() });
        readers.push(quote! { #name: ::ooxml_tools::support::field::<#ty>(map, #written, at)? });
        nested.push(quote! { <#ty as ::ooxml_tools::ArgType>::definition_problems() });
    }
    let tool = match tool_attrs(&input.attrs)? {
        Some(attrs) => tool_impl(ident, &attrs)?,
        None => TokenStream::new(),
    };
    Ok(quote! {
        impl ::ooxml_tools::ArgType for #ident {
            fn schema() -> ::ooxml_tools::schema::Schema {
                let properties = [#(#made),*].into_iter().map(|(property, _)| property).collect();
                ::ooxml_tools::schema::Schema::new(::ooxml_tools::schema::Kind::Object(
                    ::ooxml_tools::schema::ObjectNode::new(properties),
                ))
            }

            fn definition_problems() -> ::std::vec::Vec<::std::string::String> {
                let mut problems: ::std::vec::Vec<::std::string::String> =
                    [#(#made),*].into_iter().flat_map(|(_, problems)| problems).collect();
                #(problems.extend(#nested);)*
                problems
            }

            fn from_arg(
                value: &::ooxml_tools::serde_json::Value,
                at: &str,
            ) -> ::std::result::Result<Self, ::ooxml_tools::ArgError> {
                let map = ::ooxml_tools::support::object(value, at)?;
                ::std::result::Result::Ok(Self { #(#readers),* })
            }
        }
        #tool
    })
}

fn modify(chain: TokenStream, modifier: &Modifier) -> syn::Result<TokenStream> {
    Ok(match modifier {
        Modifier::Flag(name) => {
            let method = format_ident!("{name}");
            quote! { #chain.#method() }
        }
        Modifier::Bound(name, value, float) => {
            let method = format_ident!("{name}");
            let bound = if *float {
                quote! { ::ooxml_tools::schema::Bound::Float(#value) }
            } else {
                quote! { ::ooxml_tools::schema::Bound::Int(#value) }
            };
            quote! { #chain.#method(#bound) }
        }
        Modifier::Count(name, value) => {
            let method = format_ident!("{name}");
            quote! { #chain.#method(#value) }
        }
        Modifier::Str(name, value) => {
            let method = format_ident!("{name}");
            quote! { #chain.#method(#value) }
        }
        Modifier::Pair(name, a, b) => {
            let method = format_ident!("{name}");
            quote! { #chain.#method(#a, #b) }
        }
        Modifier::Kinds(name, value, kinds) => {
            let method = format_ident!("{name}");
            match value {
                Some(value) => quote! { #chain.#method(#value, &[#(#kinds),*]) },
                None => quote! { #chain.#method(&[#(#kinds),*]) },
            }
        }
        Modifier::Value(name, value) => {
            let method = format_ident!("{name}");
            let value = match number_kind(value) {
                Some(false) => quote! { ::ooxml_tools::serde_json::Value::from(#value as i64) },
                _ => quote! { ::ooxml_tools::serde_json::Value::from(#value) },
            };
            quote! { #chain.#method(#value) }
        }
        Modifier::Format(text) => {
            let format = match text.value().as_str() {
                "date" => quote! { ::ooxml_tools::schema::Format::Date },
                "date-time" => quote! { ::ooxml_tools::schema::Format::DateTime },
                _ => return Err(syn::Error::new_spanned(text, "format is \"date\" or \"date-time\"")),
            };
            quote! { #chain.format(#format) }
        }
    })
}

fn tool_impl(ident: &syn::Ident, attrs: &ToolAttrs) -> syn::Result<TokenStream> {
    let Some(name) = &attrs.name else {
        return Err(syn::Error::new_spanned(ident, "#[tool] needs name = \"...\""));
    };
    let Some(description) = &attrs.description else {
        return Err(syn::Error::new_spanned(
            ident,
            "#[tool] needs description = include_str!(\"....md\")",
        ));
    };
    let mut settings = Vec::new();
    if let Some(group) = &attrs.group {
        settings.push(quote! { spec.group = ::std::string::String::from(#group); });
    }
    for (flag, value) in &attrs.flags {
        let field = format_ident!("{flag}");
        settings.push(quote! { spec.flags.#field = #value; });
    }
    if let Some(role) = &attrs.role {
        settings.push(quote! { spec.role = ::std::option::Option::Some(::std::string::String::from(#role)); });
    }
    if let Some(route) = &attrs.route {
        settings.push(quote! { spec.route = ::std::option::Option::Some(::std::string::String::from(#route)); });
    }
    if let Some(priority) = &attrs.priority {
        settings.push(quote! { spec.priority = ::std::option::Option::Some(#priority); });
    }
    if let Some(documents) = &attrs.documents {
        settings.push(quote! { spec.documents = ::std::vec![#(::std::string::String::from(#documents)),*]; });
    }
    for group in &attrs.exactly_one {
        settings.push(quote! {
            spec.exactly_one.push(::std::vec![#(::std::string::String::from(#group)),*]);
        });
    }
    if let Some(places) = &attrs.places {
        let kind = format_ident!("{}", place_kind(places)?);
        let own = &attrs.place_own;
        let find = attrs.place_find;
        settings.push(quote! {
            spec.places = ::std::option::Option::Some(::ooxml_tools::Places::new(
                ::ooxml_tools::PlaceKind::#kind,
                &[#(#own),*],
                #find,
            ));
        });
    }
    Ok(quote! {
        impl ::ooxml_tools::ToolArgs for #ident {
            fn spec() -> ::ooxml_tools::ToolSpec {
                let mut spec = ::ooxml_tools::ToolSpec::new(
                    #name,
                    #description,
                    <Self as ::ooxml_tools::ArgType>::schema(),
                );
                #(#settings)*
                spec
            }
        }
    })
}

/// The `ooxml_tools::PlaceKind` variant `places = "..."` names.
fn place_kind(name: &LitStr) -> syn::Result<&'static str> {
    Ok(match name.value().as_str() {
        "blocks" => "Blocks",
        "text" => "Text",
        "mark" => "Mark",
        "picture" => "Picture",
        "sides" => "Sides",
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "places is blocks, text, mark, picture or sides",
            ))
        }
    })
}

/// `ArgEnum` and `ArgType` for an enum of unit variants.
pub fn arg_enum(input: &DeriveInput) -> syn::Result<TokenStream> {
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "ArgEnum needs an enum of unit variants",
        ));
    };
    let ident = &input.ident;
    let mut variants = Vec::new();
    let mut values = Vec::new();
    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(variant, "ArgEnum variants take no fields"));
        }
        let attrs = field_attrs(&variant.attrs)?;
        let value = attrs
            .rename
            .as_ref()
            .map_or_else(|| snake_case(&variant.ident.to_string()), LitStr::value);
        variants.push(variant.ident.clone());
        values.push(value);
    }
    Ok(quote! {
        impl ::ooxml_tools::ArgEnum for #ident {
            const VALUES: &'static [&'static str] = &[#(#values),*];

            fn as_str(self) -> &'static str {
                match self { #(Self::#variants => #values),* }
            }

            fn from_str_value(text: &str) -> ::std::option::Option<Self> {
                match text {
                    #(#values => ::std::option::Option::Some(Self::#variants),)*
                    _ => ::std::option::Option::None,
                }
            }
        }

        impl ::ooxml_tools::ArgType for #ident {
            fn schema() -> ::ooxml_tools::schema::Schema {
                ::ooxml_tools::schema::Schema::one_of(<Self as ::ooxml_tools::ArgEnum>::VALUES)
            }

            fn from_arg(
                value: &::ooxml_tools::serde_json::Value,
                at: &str,
            ) -> ::std::result::Result<Self, ::ooxml_tools::ArgError> {
                value
                    .as_str()
                    .and_then(<Self as ::ooxml_tools::ArgEnum>::from_str_value)
                    .ok_or_else(|| ::ooxml_tools::ArgError::new(at, "one of the enum's values"))
            }
        }
    })
}

/// `CamelCase` as `camel_case`.
fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (index, c) in name.chars().enumerate() {
        if c.is_uppercase() && index > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

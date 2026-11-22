//! Owns: a document's `check` given a deck's arguments, as docx-agent's `_check` reads them
//! (round 23, V3): `slides` read as the document's `pages` (with a warning saying so; ignored
//! beside `pages`), and `boxes`, `spacing`, `contrast` and a deck's facts in `include`
//! refused as a deck's, naming what a document takes instead.
//!
//! Must not: check anything; the call's other arguments are [`super::check`]'s.

use super::args::{Check, CheckInclude};
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{ArgEnum, IntOrStr, Refusal, ToolError, TryCall, WayOut};
use serde_json::{json, Map, Value};
use std::borrow::Cow;

/// What a document reports (the way out of a deck's facts in `include`).
const DOCUMENT_PARTS: [&str; 4] = ["validate", "reflow", "fields", "layout"];

/// `args` as a document's check reads them, and the warning that says how `slides` was
/// read, if it was given.
///
/// # Errors
/// `invalid_arguments` for a slide that is no page number, and for a deck's argument.
pub(super) fn read(args: &Check) -> Result<(Cow<'_, Check>, Option<&'static str>), ToolError> {
    let (args, warning) = match &args.slides {
        None => (Cow::Borrowed(args), None),
        Some(_) if args.pages.is_some() => (
            Cow::Owned(Check {
                slides: None,
                ..args.clone()
            }),
            Some("slides ignored: a document has pages"),
        ),
        Some(slides) => {
            let pages = slides
                .iter()
                .enumerate()
                .map(|(index, slide)| page(index, slide, &args.doc))
                .collect::<Result<Vec<_>, _>>()?;
            (
                Cow::Owned(Check {
                    slides: None,
                    pages: Some(pages),
                    ..args.clone()
                }),
                Some("slides read as pages: a document has pages"),
            )
        }
    };
    for (name, given) in [
        ("boxes", args.boxes.is_some()),
        ("spacing", args.spacing.is_some()),
        ("contrast", args.contrast.is_some()),
    ] {
        if given {
            return Err(ToolError::refusal(
                Refusal::InvalidArguments,
                format!("{name} is for decks; for a document use include"),
                WayOut::call(again(&args.doc)),
            )
            .at(name));
        }
    }
    let deck: Vec<&str> = args
        .include
        .iter()
        .flatten()
        .filter(|part| {
            matches!(
                part,
                CheckInclude::Fit | CheckInclude::Collisions | CheckInclude::Facts | CheckInclude::Design
            )
        })
        .map(|part| part.as_str())
        .collect();
    if !deck.is_empty() {
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            format!(
                "{} are deck facts; a document reports validate, reflow, fields and layout",
                deck.join(", ")
            ),
            WayOut::names(DOCUMENT_PARTS).unwrap_or_else(|| WayOut::call(again(&args.doc))),
        )
        .at("include"));
    }
    Ok((args, warning))
}

/// A slide read as a page: a number, or a string of digits (a deck's id is none).
fn page(index: usize, slide: &IntOrStr<1>, doc: &str) -> Result<i64, ToolError> {
    match slide {
        IntOrStr::Int(n) => Ok(*n),
        IntOrStr::Str(text) => text
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|_| digits(text.trim()))
            .ok_or_else(|| {
                ToolError::refusal(
                    Refusal::InvalidArguments,
                    format!(
                        "slides[{index}] is {}: a document has pages, numbered from 1 (pages: [1])",
                        repr_str(text)
                    ),
                    WayOut::call(TryCall {
                        tool: "check".into(),
                        arguments: arguments(doc, Some(json!([1]))),
                        fill: vec!["pages".into()],
                    }),
                )
                .at("pages")
            }),
    }
}

/// Python's `str.isdigit` on ASCII: one digit at least, digits only.
fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// The check of the document with none of a deck's arguments.
fn again(doc: &str) -> TryCall {
    TryCall {
        tool: "check".into(),
        arguments: arguments(doc, None),
        fill: Vec::new(),
    }
}

fn arguments(doc: &str, pages: Option<Value>) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("doc".into(), Value::from(doc));
    if let Some(pages) = pages {
        out.insert("pages".into(), pages);
    }
    out
}

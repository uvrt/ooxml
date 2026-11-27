//! Owns: an address read once, where a tool takes it ([`Address`]): the form it is written
//! in ([`Form`]), every form its text could also be read as ([`Shape`], for the resolvers
//! that try forms in their own order), and the text as given, which every refusal quotes;
//! and where new blocks go ([`Place`]: `end`, `end:<story>`, `after:`, `before:`,
//! `replace:`, a bare address).
//!
//! Reading is purely syntactic and never fails: a malformed span is a form of its own
//! ([`Form::Malformed`]), refused where the resolver met it before, so a refusal's wording
//! and order stay what they were. Whether a bare name is a story or a bookmark is the
//! document's to say ([`Form::Bare`]).
//!
//! Must not: look at a document; whether an address names anything is
//! [`mod@super::resolve`]'s.

use super::resolve::HEADING;
use super::syntax::{inside_table, is_text_range, parse_range, span_ends, Ends, Inside, RangeText};
use super::AddressError;
use std::sync::Arc;

/// An address as a tool was given it, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    raw: Arc<str>,
    form: Form,
    shapes: u16,
}

/// The form an address is written in: the first of its [`Shape`]s in the order the
/// address resolver tries them ([`super::scope_paragraphs`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Form {
    /// Blank.
    Empty,
    /// A heading's range (`h:p:A`), or a span with one at an end (`h:p:A..p:B`).
    Heading,
    /// A span of blocks (`p:A..t:B`), either end open.
    Span {
        /// Its first block, `None` for an open start.
        first: Option<Arc<str>>,
        /// Its last block, `None` for an open end.
        last: Option<Arc<str>>,
    },
    /// A malformed span (`p:A...p:B`, `..`, `p:A@3..p:B`), with its refusal.
    Malformed(AddressError),
    /// A text position or range (`p:A@5`, `p:A@4:11`, `p:A@3..p:B@8`).
    Text,
    /// A section (`s:2`, `s:<paraId>`, `s:body`, `s@...`).
    Section,
    /// A bookmark by its `bm:` name.
    Bookmark,
    /// A name without `:`, `@`, `/` or `#`: a story (`body`, `header1`) or a bookmark.
    Bare,
    /// A content control (`cc:`, `cc@`).
    Control,
    /// A row by its id (`tr:`, `header1/tr:`).
    Row,
    /// A row or cell below a table (`t:X/r1`, `t:X/c1,2`).
    Inside {
        /// The table, as written.
        table: Arc<str>,
        /// The row or cell.
        inside: Inside,
    },
    /// Anything else: a block's id (`p:A`, `t:B`, `header1/p:...`), or a name
    /// that names nothing.
    Block,
}

/// A form an address's text can be read as; an address may have several (`s:1@5` is a
/// section's and a text position's), and each resolver tries them in its own order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Shape {
    /// Blank.
    Empty = 1,
    /// A heading's range, or a span with one at an end.
    Heading = 1 << 1,
    /// `..` in it: a span of blocks, a malformed one, or a text range across paragraphs.
    Dots = 1 << 2,
    /// A text position or range.
    Text = 1 << 3,
    /// `s:` or `s@` first.
    Section = 1 << 4,
    /// `bm:` first.
    Bookmark = 1 << 5,
    /// No `:`, `@`, `/` or `#`.
    Bare = 1 << 6,
    /// `cc:` or `cc@` first.
    Control = 1 << 7,
    /// `tr:` first, or `/tr:` in it.
    Row = 1 << 8,
    /// A row or cell below a table (`t:X/r1`, `t:X/c1,2`).
    Inside = 1 << 9,
    /// No `:`, `@` or `/`: a story's name could be it.
    Story = 1 << 10,
}

impl Address {
    /// `raw` read.
    #[must_use]
    pub fn parse(raw: &str) -> Address {
        Address::from_arc(Arc::from(raw))
    }

    /// `raw` read, its text shared.
    #[must_use]
    pub fn from_arc(raw: Arc<str>) -> Address {
        let shapes = shapes_of(&raw);
        let form = form_of(&raw, shapes);
        Address { raw, form, shapes }
    }

    /// The span `first..last` (a heading's range as the span it is): its text as the
    /// resolver has always written it.
    #[must_use]
    pub fn span(first: &str, last: &str) -> Address {
        Address::parse(&format!("{first}..{last}"))
    }

    /// The text as given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// The text as given, shared.
    #[must_use]
    pub fn raw(&self) -> &Arc<str> {
        &self.raw
    }

    /// The form it is written in.
    #[must_use]
    pub fn form(&self) -> &Form {
        &self.form
    }

    /// Whether its text can be read as `shape`.
    #[must_use]
    pub fn has(&self, shape: Shape) -> bool {
        self.shapes & shape as u16 != 0
    }

    /// Its span's ends (`None` for an open one), or `None` when it is not a span of
    /// blocks; the refusal of a malformed one.
    ///
    /// # Errors
    /// A malformed span, with the syntax.
    pub fn span_ends(&self) -> Result<Option<Ends<'_>>, AddressError> {
        match &self.form {
            Form::Span { first, last } => Ok(Some((first.as_deref(), last.as_deref()))),
            Form::Malformed(error) => Err(error.clone()),
            // a heading's range is read as the span it is written as, as before
            _ if self.has(Shape::Dots) => span_ends(&self.raw),
            _ => Ok(None),
        }
    }

    /// The text range it is written as (`p:A@4:11`, `p:A@5`, `p:A@3..p:B@8`), if any.
    #[must_use]
    pub fn text_range(&self) -> Option<RangeText> {
        parse_range(&self.raw)
    }

    /// The table and the row or cell below it it names (`t:X/c1,2`, `t:X/r1`), if any.
    #[must_use]
    pub fn inside_table(&self) -> Option<(&str, Inside)> {
        match &self.form {
            Form::Inside { table, inside } => Some((table, *inside)),
            _ if self.has(Shape::Inside) => inside_table(&self.raw),
            _ => None,
        }
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.raw)
    }
}

impl From<&str> for Address {
    fn from(raw: &str) -> Self {
        Address::parse(raw)
    }
}

impl From<String> for Address {
    fn from(raw: String) -> Self {
        Address::from_arc(Arc::from(raw))
    }
}

/// Whether a heading's range is in `raw` where the resolver expands one: the whole address,
/// or an end of a span.
fn heading_at_end(raw: &str) -> bool {
    if !raw.contains(HEADING) {
        return false;
    }
    match raw.split_once("..") {
        None => raw.starts_with(HEADING),
        Some((first, last)) => first.trim().starts_with(HEADING) || last.trim().starts_with(HEADING),
    }
}

fn shapes_of(raw: &str) -> u16 {
    let mut shapes = 0;
    let mut add = |shape: Shape, yes: bool| {
        if yes {
            shapes |= shape as u16;
        }
    };
    add(Shape::Empty, raw.trim().is_empty());
    add(Shape::Heading, heading_at_end(raw));
    add(Shape::Dots, raw.contains(".."));
    add(Shape::Text, is_text_range(raw));
    add(Shape::Section, raw.starts_with("s:") || raw.starts_with("s@"));
    add(Shape::Bookmark, raw.starts_with("bm:"));
    add(Shape::Bare, !raw.contains([':', '@', '/', '#']));
    add(Shape::Control, raw.starts_with("cc:") || raw.starts_with("cc@"));
    add(Shape::Row, raw.starts_with("tr:") || raw.contains("/tr:"));
    add(Shape::Inside, inside_table(raw).is_some());
    add(Shape::Story, !raw.contains([':', '@', '/']));
    shapes
}

/// The first form of `raw`'s, in the address resolver's order.
fn form_of(raw: &str, shapes: u16) -> Form {
    let has = |shape: Shape| shapes & shape as u16 != 0;
    if has(Shape::Empty) {
        return Form::Empty;
    }
    if has(Shape::Heading) {
        return Form::Heading;
    }
    match span_ends(raw) {
        Err(error) => return Form::Malformed(error),
        Ok(Some((first, last))) => {
            return Form::Span {
                first: first.map(Arc::from),
                last: last.map(Arc::from),
            }
        }
        Ok(None) => {}
    }
    if has(Shape::Text) {
        Form::Text
    } else if has(Shape::Section) {
        Form::Section
    } else if has(Shape::Bookmark) {
        Form::Bookmark
    } else if has(Shape::Bare) {
        Form::Bare
    } else if has(Shape::Control) {
        Form::Control
    } else if has(Shape::Row) {
        Form::Row
    } else if let Some((table, inside)) = inside_table(raw) {
        Form::Inside {
            table: Arc::from(table),
            inside,
        }
    } else {
        Form::Block
    }
}

/// Where new blocks go, as a tool was given it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    raw: Arc<str>,
    kind: PlaceKind,
}

/// The kinds of [`Place`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceKind {
    /// A story's end (`end`, `end:<story>`; `None`: the body).
    End(Option<Arc<str>>),
    /// After a block (`after:<id>`).
    After(Address),
    /// Before a block (`before:<id>`).
    Before(Address),
    /// In place of blocks (`replace:<id>`, `replace:<id>..<id>`).
    Replace(Address),
    /// A bare address: after the block it names.
    At(Address),
}

impl Place {
    /// `raw` read (docx-agent's `_locate`): `end` or `end:<story>`, `after:`, `before:`,
    /// `replace:`, else a bare address.
    #[must_use]
    pub fn parse(raw: &str) -> Place {
        let kind = match raw.split_once(':') {
            Some(("end", story)) => PlaceKind::End((!story.is_empty()).then(|| Arc::from(story))),
            Some(("after", rest)) => PlaceKind::After(Address::parse(rest)),
            Some(("before", rest)) => PlaceKind::Before(Address::parse(rest)),
            Some(("replace", rest)) => PlaceKind::Replace(Address::parse(rest)),
            _ if raw == "end" => PlaceKind::End(None),
            _ => PlaceKind::At(Address::parse(raw)),
        };
        Place {
            raw: Arc::from(raw),
            kind,
        }
    }

    /// The text as given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// What it is.
    #[must_use]
    pub fn kind(&self) -> &PlaceKind {
        &self.kind
    }

    /// Whether it is the body's end (`end`, `end:body`).
    #[must_use]
    pub fn is_body_end(&self) -> bool {
        matches!(&*self.raw, "end" | "end:body")
    }
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.raw)
    }
}

impl From<&str> for Place {
    fn from(raw: &str) -> Self {
        Place::parse(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(raw: &str) -> Form {
        Address::parse(raw).form().clone()
    }

    #[test]
    fn each_form_reads_as_the_resolver_tries_it() {
        assert_eq!(form("  "), Form::Empty);
        assert_eq!(form("h:p:A"), Form::Heading);
        assert_eq!(form("p:A..h:p:B"), Form::Heading);
        assert_eq!(
            form("p:A.."),
            Form::Span {
                first: Some(Arc::from("p:A")),
                last: None
            }
        );
        assert!(matches!(form("p:A...p:B"), Form::Malformed(_)));
        assert_eq!(form("p:A@3..p:B@8"), Form::Text);
        assert_eq!(form("p:A@4:11"), Form::Text);
        assert_eq!(form("s:2"), Form::Section);
        assert_eq!(form("bm:Intro"), Form::Bookmark);
        assert_eq!(form("Intro"), Form::Bare);
        assert_eq!(form("cc:101"), Form::Control);
        assert_eq!(form("tr:0A0B0C0D"), Form::Row);
        assert_eq!(
            form("t:X/c1,2"),
            Form::Inside {
                table: Arc::from("t:X"),
                inside: Inside::Cell(1, 2)
            }
        );
        assert_eq!(form("p@body/7"), Form::Block);
        assert_eq!(form("header1/p:1A2B3C4D"), Form::Block);
        // a section's text and a text position's both: a resolver trying sections first sees it
        let both = Address::parse("s:1@5");
        assert_eq!(both.form(), &Form::Text);
        assert!(both.has(Shape::Section));
        assert_eq!(both.as_str(), "s:1@5");
    }

    #[test]
    fn a_place_reads_as_locate_reads_it() {
        assert_eq!(Place::parse("end").kind(), &PlaceKind::End(None));
        assert_eq!(Place::parse("end:").kind(), &PlaceKind::End(None));
        assert_eq!(
            Place::parse("end:header1").kind(),
            &PlaceKind::End(Some(Arc::from("header1")))
        );
        assert_eq!(
            Place::parse("after:p:A").kind(),
            &PlaceKind::After(Address::parse("p:A"))
        );
        assert_eq!(
            Place::parse("before:t:B").kind(),
            &PlaceKind::Before(Address::parse("t:B"))
        );
        assert_eq!(
            Place::parse("replace:p:A..p:B").kind(),
            &PlaceKind::Replace(Address::parse("p:A..p:B"))
        );
        assert_eq!(Place::parse("p:A").kind(), &PlaceKind::At(Address::parse("p:A")));
        assert!(Place::parse("end").is_body_end());
        assert!(Place::parse("end:body").is_body_end());
        assert!(!Place::parse("end:").is_body_end());
        assert!(!Place::parse("end:header1").is_body_end());
    }
}

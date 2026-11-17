//! Owns: the read half of docx-agent's style map (`markdown/stylemap.py`, its default
//! rules): which Markdown construct a Word style stands for, by its name in any case --
//! headings by name and then by outline level, paragraph and character rules by name.
//!
//! Must not: write styles (the write half is phase P2's).

use crate::docx::casefold;

/// A construct a style stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Construct {
    /// A body paragraph.
    Paragraph,
    /// A heading of a level.
    Heading(i64),
    /// A bullet list of a level (from 1).
    Bullet(i64),
    /// A numbered list of a level (from 1).
    Ordered(i64),
    /// A block quote.
    Quote,
    /// A code block.
    CodeBlock,
    /// A caption (written only).
    Caption,
    /// Inline code (a character style).
    Code,
    /// Any other character construct (emphasis, strong, link, note reference).
    OtherCharacter,
}

/// The default rules' names by kind: `(paragraph?, name, construct)`, first rule wins.
fn rules() -> Vec<(bool, String, Construct)> {
    let mut out: Vec<(bool, String, Construct)> = Vec::new();
    let paragraph = |name: &str, c| (true, name.to_owned(), c);
    for name in [
        "Normal",
        "Body Text",
        "First Paragraph",
        "Compact",
        "List Paragraph",
        "footnote text",
        "endnote text",
        "annotation text",
        "header",
        "footer",
    ] {
        out.push(paragraph(name, Construct::Paragraph));
    }
    for n in 1..=6 {
        out.push(paragraph(&format!("heading {n}"), Construct::Heading(n)));
    }
    out.push(paragraph("List Bullet", Construct::Bullet(1)));
    for n in 2..=5 {
        out.push(paragraph(&format!("List Bullet {n}"), Construct::Bullet(n)));
    }
    out.push(paragraph("List Number", Construct::Ordered(1)));
    for n in 2..=5 {
        out.push(paragraph(&format!("List Number {n}"), Construct::Ordered(n)));
    }
    out.push(paragraph("Quote", Construct::Quote));
    out.push(paragraph("Block Text", Construct::Quote));
    out.push(paragraph("HTML Preformatted", Construct::CodeBlock));
    out.push(paragraph("Source Code", Construct::CodeBlock));
    for (name, c) in [
        ("HTML Code", Construct::Code),
        ("Verbatim Char", Construct::Code),
        ("Emphasis", Construct::OtherCharacter),
        ("Strong", Construct::OtherCharacter),
        ("Hyperlink", Construct::OtherCharacter),
        ("footnote reference", Construct::OtherCharacter),
        ("endnote reference", Construct::OtherCharacter),
    ] {
        out.push((false, name.to_owned(), c));
    }
    // The table-cell and footnote-text rules name Normal and footnote text, already taken.
    out.push(paragraph("caption", Construct::Caption));
    out
}

/// The default style map's read half.
#[derive(Debug, Clone)]
pub struct StyleMap {
    rules: Vec<(bool, String, Construct)>,
}

impl Default for StyleMap {
    fn default() -> Self {
        StyleMap { rules: rules() }
    }
}

impl StyleMap {
    fn find(&self, paragraph: bool, name: Option<&str>) -> Option<Construct> {
        let wanted = casefold(name.filter(|n| !n.is_empty())?);
        self.rules
            .iter()
            .find(|(p, n, _)| *p == paragraph && casefold(n) == wanted)
            .map(|(_, _, c)| *c)
    }

    /// The rule a paragraph style's name names.
    #[must_use]
    pub fn paragraph_rule(&self, name: Option<&str>) -> Option<Construct> {
        self.find(true, name)
    }

    /// The rule a character style's name names.
    #[must_use]
    pub fn character_rule(&self, name: Option<&str>) -> Option<Construct> {
        self.find(false, name)
    }

    /// A paragraph's heading level: its style's rule, else `heading N` by name, else its
    /// style chain's outline level (0-8) plus one.
    #[must_use]
    pub fn heading_level(&self, name: Option<&str>, outline_level: Option<i64>) -> Option<i64> {
        if let Some(Construct::Heading(level)) = self.paragraph_rule(name) {
            return Some(level);
        }
        let lower = name.unwrap_or("").to_lowercase();
        if let Some(d) = lower.strip_prefix("heading ") {
            if let [b @ b'1'..=b'9'] = d.as_bytes() {
                return Some(i64::from(b - b'0'));
            }
        }
        outline_level.filter(|l| (0..=8).contains(l)).map(|l| l + 1)
    }
}

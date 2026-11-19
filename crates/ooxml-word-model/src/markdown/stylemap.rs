//! Owns: the read half of docx-agent's style map (`markdown/stylemap.py`, its default
//! rules): which Markdown construct a Word style stands for, by its name in any case --
//! headings by name and then by outline level, paragraph and character rules by name.
//!
//! The write half ([`WriteMap`]): the style each construct is written in, the default
//! table's or one a caller's map names in its place (`StyleMap.from_dict`, `style_for`).
//!
//! Must not: write styles (ooxml-word-edit applies the write half).

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

/// What kind of style a construct is written with (`STYLE_KINDS`).
#[must_use]
pub fn construct_kind(construct: &str) -> &'static str {
    match construct {
        "code" | "emphasis" | "strong" | "link" | "footnote_reference" => "character",
        "table" => "table",
        "strikethrough" | "thematic_break" => "direct",
        _ => "paragraph",
    }
}

/// One row of the write half: `construct` (at `level` for headings and lists) is written in
/// the style named `style` (none: no style).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRule {
    /// The construct (`paragraph`, `heading`, `bullet_list`, `table_cell_center`...).
    pub construct: String,
    /// The level, from 1 for headings and lists, else 0.
    pub level: i64,
    /// The style's name (`w:name`, an alias or an id), or none.
    pub style: Option<String>,
}

/// The write half of the style map: the rows, first match wins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteMap {
    rules: Vec<WriteRule>,
}

fn write_rules() -> Vec<WriteRule> {
    let rule = |construct: &str, level: i64, style: Option<&str>| WriteRule {
        construct: construct.to_owned(),
        level,
        style: style.map(str::to_owned),
    };
    let mut out = vec![rule("paragraph", 0, Some("Normal"))];
    for n in 1..=6 {
        out.push(rule("heading", n, Some(&format!("heading {n}"))));
    }
    out.push(rule("bullet_list", 1, Some("List Bullet")));
    for n in 2..=5 {
        out.push(rule("bullet_list", n, Some(&format!("List Bullet {n}"))));
    }
    out.push(rule("ordered_list", 1, Some("List Number")));
    for n in 2..=5 {
        out.push(rule("ordered_list", n, Some(&format!("List Number {n}"))));
    }
    for (construct, style) in [
        ("blockquote", Some("Quote")),
        ("code_block", Some("HTML Preformatted")),
        ("code", Some("HTML Code")),
        ("emphasis", Some("Emphasis")),
        ("strong", Some("Strong")),
        ("strikethrough", None),
        ("link", Some("Hyperlink")),
        ("table", Some("Table Grid")),
        ("table_cell", Some("Normal")),
        ("table_cell_center", None),
        ("table_cell_right", None),
        ("footnote_text", Some("footnote text")),
        ("footnote_reference", Some("footnote reference")),
        ("thematic_break", None),
        ("caption", Some("caption")),
    ] {
        out.push(rule(construct, 0, style));
    }
    out
}

/// A `style_map` key's construct and level (`DICT_KEYS`, with `_DICT_ALIASES`).
#[must_use]
pub fn dict_key(key: &str) -> Option<(&'static str, i64)> {
    let name = key.trim().to_lowercase();
    let name = name.as_str();
    let level = |prefixes: &[&str]| -> Option<i64> {
        prefixes.iter().find_map(|p| {
            let rest = name.strip_prefix(p)?;
            let n: i64 = rest.parse().ok()?;
            (rest.len() == 1).then_some(n)
        })
    };
    if let Some(n) = level(&["h", "heading", "heading ", "heading_"]).filter(|n| (1..=6).contains(n)) {
        return Some(("heading", n));
    }
    if let Some(n) = level(&["bullet"]).filter(|n| (1..=5).contains(n)) {
        return Some(("bullet_list", n));
    }
    if let Some(n) = level(&["number"]).filter(|n| (1..=5).contains(n)) {
        return Some(("ordered_list", n));
    }
    Some(match name {
        "paragraph" | "body" => ("paragraph", 0),
        "quote" | "blockquote" => ("blockquote", 0),
        "code" | "code_block" => ("code_block", 0),
        "inline_code" => ("code", 0),
        "bullet" | "bullet_list" => ("bullet_list", 1),
        "number" | "ordered_list" => ("ordered_list", 1),
        "table" => ("table", 0),
        "table_cell" | "cell" => ("table_cell", 0),
        "table_cell_center" => ("table_cell_center", 0),
        "table_cell_right" => ("table_cell_right", 0),
        "footnote" | "footnote_text" => ("footnote_text", 0),
        "footnote_reference" => ("footnote_reference", 0),
        "emphasis" => ("emphasis", 0),
        "strong" => ("strong", 0),
        "link" => ("link", 0),
        "caption" => ("caption", 0),
        _ => return None,
    })
}

impl Default for WriteMap {
    fn default() -> Self {
        WriteMap { rules: write_rules() }
    }
}

impl WriteMap {
    /// The default table with the rows `pairs` name replaced (`StyleMap.from_dict`): each
    /// key a [`dict_key`], each value a style's name or none.
    ///
    /// # Errors
    /// The first key no construct has, with the keys there are.
    pub fn from_pairs(pairs: &[(String, Option<String>)]) -> Result<WriteMap, String> {
        let mut given: Vec<WriteRule> = Vec::new();
        for (key, value) in pairs {
            let (construct, level) = dict_key(key).ok_or_else(|| {
                format!(
                    "unknown style_map key {key:?}: the keys are paragraph, h1-h6, quote, code, inline_code, \
                     bullet, bullet2-bullet5, number, number2-number5, table, table_cell, table_cell_center, \
                     table_cell_right, footnote, footnote_reference, emphasis, strong, link, caption"
                )
            })?;
            given.push(WriteRule {
                construct: construct.to_owned(),
                level,
                style: value.clone(),
            });
        }
        let keys: Vec<(String, i64)> = given.iter().map(|r| (r.construct.clone(), r.level)).collect();
        let rest = write_rules()
            .into_iter()
            .filter(|r| !keys.contains(&(r.construct.clone(), r.level)));
        given.extend(rest);
        Ok(WriteMap { rules: given })
    }

    /// The rule written for `construct` at `level`: the exact level's row, else the deepest
    /// row below it, else its first row.
    #[must_use]
    pub fn style_for(&self, construct: &str, level: i64) -> Option<&WriteRule> {
        let rows: Vec<&WriteRule> = self.rules.iter().filter(|r| r.construct == construct).collect();
        if let Some(exact) = rows.iter().find(|r| r.level == level) {
            return Some(exact);
        }
        rows.iter()
            .filter(|r| r.level <= level)
            .max_by_key(|r| r.level)
            .or_else(|| rows.first())
            .copied()
    }

    /// Whether this is the default table's row for `construct` at `level`.
    #[must_use]
    pub fn is_default(&self, construct: &str, level: i64) -> bool {
        self.style_for(construct, level) == WriteMap::default().style_for(construct, level)
    }
}

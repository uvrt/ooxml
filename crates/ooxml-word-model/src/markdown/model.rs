//! Owns: the neutral Markdown model the reader writes and the renderer prints
//! (docx-agent's `markdown/model.py`): blocks and inline items, plain data.
//!
//! Must not: read a document or write text.

/// An inline mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mark {
    /// `~~`.
    Strike,
    /// `**`.
    Strong,
    /// `*`.
    Em,
    /// Inline code.
    Code,
}

/// A set of marks.
pub type Marks = std::collections::BTreeSet<Mark>;

/// An inline item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Literal text with marks and, inside a link, its target.
    Text {
        /// The text.
        text: String,
        /// Its marks.
        marks: Marks,
        /// The link it is in.
        href: Option<String>,
    },
    /// Raw inline HTML (a comment).
    Html(String),
    /// A CriticMarkup delimiter.
    Critic(String),
    /// A hard line break.
    Break,
    /// A picture.
    Image {
        /// Its alternative text.
        alt: String,
        /// Its source (the drawing id).
        src: String,
        /// Its title (the drawing's name).
        title: Option<String>,
        /// The link it is in.
        href: Option<String>,
    },
    /// A footnote reference.
    NoteRef(String),
}

impl Inline {
    /// Plain text without marks.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Inline {
        Inline::Text {
            text: text.into(),
            marks: Marks::new(),
            href: None,
        }
    }
}

/// A block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// An HTML comment on a line of its own.
    Comment(String),
    /// A paragraph.
    Paragraph(Vec<Inline>),
    /// A heading.
    Heading(i64, Vec<Inline>),
    /// A fenced code block.
    Code(String),
    /// A block quote.
    Quote(Vec<Block>),
    /// A list.
    List(ListBlock),
    /// A GFM table.
    Table(Table),
    /// Raw HTML on lines of its own.
    Html(String),
    /// A thematic break.
    Rule,
    /// A footnote definition.
    Footnote(String, Vec<Block>),
}

/// A list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListBlock {
    /// Numbered.
    pub ordered: bool,
    /// The first number.
    pub start: i64,
    /// Each item's blocks.
    pub items: Vec<Vec<Block>>,
    /// Written with the other marker (`*`, `)`), to part it from the list before.
    pub alternate: bool,
}

/// A GFM table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The header row's cells.
    pub header: Vec<Vec<Inline>>,
    /// The other rows.
    pub rows: Vec<Vec<Vec<Inline>>>,
    /// Each column's alignment (`center`, `right`).
    pub aligns: Vec<Option<String>>,
    /// Whether the first row is a header row.
    pub headed: bool,
}

/// The characters a reader sees: text and breaks.
#[must_use]
pub fn plain_text(inline: &[Inline]) -> String {
    let mut out = String::new();
    for item in inline {
        match item {
            Inline::Text { text, .. } => out.push_str(text),
            Inline::Break => out.push('\n'),
            _ => {}
        }
    }
    out
}

/// Whether `inline` writes anything a Markdown reader keeps.
#[must_use]
pub fn has_content(inline: &[Inline]) -> bool {
    inline.iter().any(|i| match i {
        Inline::Text { text, .. } => !text.trim().is_empty(),
        Inline::Html(_) | Inline::Critic(_) | Inline::Image { .. } | Inline::NoteRef(_) => true,
        Inline::Break => false,
    })
}

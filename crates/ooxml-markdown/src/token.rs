//! Owns: [`Token`], one item of markdown-it's token stream, its [`Kind`] and attributes.
//!
//! Must not: parse.

/// What a token is (markdown-it's `type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Text.
    Text,
    /// An escaped character (`\*`), joined into the text around it once parsing ends.
    TextSpecial,
    /// A hard line break.
    Hardbreak,
    /// A soft line break.
    Softbreak,
    /// `<strong>` opens.
    StrongOpen,
    /// `</strong>`.
    StrongClose,
    /// `<em>` opens.
    EmOpen,
    /// `</em>`.
    EmClose,
    /// `<s>` opens (`~~`).
    SOpen,
    /// `</s>`.
    SClose,
    /// A code span.
    CodeInline,
    /// A link opens.
    LinkOpen,
    /// A link closes.
    LinkClose,
    /// An image.
    Image,
    /// Inline HTML.
    HtmlInline,
    /// A footnote reference (`[^label]`).
    FootnoteRef,
    /// A paragraph opens.
    ParagraphOpen,
    /// A paragraph closes.
    ParagraphClose,
    /// A heading opens.
    HeadingOpen,
    /// A heading closes.
    HeadingClose,
    /// A block's inline content, parsed into its children.
    Inline,
    /// An indented code block.
    CodeBlock,
    /// A fenced code block.
    Fence,
    /// A block quote opens.
    BlockquoteOpen,
    /// A block quote closes.
    BlockquoteClose,
    /// A thematic break.
    Hr,
    /// A bullet list opens.
    BulletListOpen,
    /// A bullet list closes.
    BulletListClose,
    /// An ordered list opens.
    OrderedListOpen,
    /// An ordered list closes.
    OrderedListClose,
    /// A list item opens.
    ListItemOpen,
    /// A list item closes.
    ListItemClose,
    /// An HTML block.
    HtmlBlock,
    /// A table opens.
    TableOpen,
    /// A table closes.
    TableClose,
    /// The head rows open.
    TheadOpen,
    /// The head rows close.
    TheadClose,
    /// The body rows open.
    TbodyOpen,
    /// The body rows close.
    TbodyClose,
    /// A row opens.
    TrOpen,
    /// A row closes.
    TrClose,
    /// A head cell opens.
    ThOpen,
    /// A head cell closes.
    ThClose,
    /// A cell opens.
    TdOpen,
    /// A cell closes.
    TdClose,
    /// A footnote definition opens while it is read (later moved to the footnote block).
    FootnoteReferenceOpen,
    /// It closes.
    FootnoteReferenceClose,
    /// The footnotes' block opens.
    FootnoteBlockOpen,
    /// It closes.
    FootnoteBlockClose,
    /// A footnote opens.
    FootnoteOpen,
    /// A footnote closes.
    FootnoteClose,
    /// The back reference at a footnote's end.
    FootnoteAnchor,
}

impl Kind {
    /// markdown-it's name for it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::TextSpecial => "text_special",
            Kind::Hardbreak => "hardbreak",
            Kind::Softbreak => "softbreak",
            Kind::StrongOpen => "strong_open",
            Kind::StrongClose => "strong_close",
            Kind::EmOpen => "em_open",
            Kind::EmClose => "em_close",
            Kind::SOpen => "s_open",
            Kind::SClose => "s_close",
            Kind::CodeInline => "code_inline",
            Kind::LinkOpen => "link_open",
            Kind::LinkClose => "link_close",
            Kind::Image => "image",
            Kind::HtmlInline => "html_inline",
            Kind::FootnoteRef => "footnote_ref",
            Kind::ParagraphOpen => "paragraph_open",
            Kind::ParagraphClose => "paragraph_close",
            Kind::HeadingOpen => "heading_open",
            Kind::HeadingClose => "heading_close",
            Kind::Inline => "inline",
            Kind::CodeBlock => "code_block",
            Kind::Fence => "fence",
            Kind::BlockquoteOpen => "blockquote_open",
            Kind::BlockquoteClose => "blockquote_close",
            Kind::Hr => "hr",
            Kind::BulletListOpen => "bullet_list_open",
            Kind::BulletListClose => "bullet_list_close",
            Kind::OrderedListOpen => "ordered_list_open",
            Kind::OrderedListClose => "ordered_list_close",
            Kind::ListItemOpen => "list_item_open",
            Kind::ListItemClose => "list_item_close",
            Kind::HtmlBlock => "html_block",
            Kind::TableOpen => "table_open",
            Kind::TableClose => "table_close",
            Kind::TheadOpen => "thead_open",
            Kind::TheadClose => "thead_close",
            Kind::TbodyOpen => "tbody_open",
            Kind::TbodyClose => "tbody_close",
            Kind::TrOpen => "tr_open",
            Kind::TrClose => "tr_close",
            Kind::ThOpen => "th_open",
            Kind::ThClose => "th_close",
            Kind::TdOpen => "td_open",
            Kind::TdClose => "td_close",
            Kind::FootnoteReferenceOpen => "footnote_reference_open",
            Kind::FootnoteReferenceClose => "footnote_reference_close",
            Kind::FootnoteBlockOpen => "footnote_block_open",
            Kind::FootnoteBlockClose => "footnote_block_close",
            Kind::FootnoteOpen => "footnote_open",
            Kind::FootnoteClose => "footnote_close",
            Kind::FootnoteAnchor => "footnote_anchor",
        }
    }
}

/// An attribute's value: text, or a number (an ordered list's `start`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Text.
    Text(String),
    /// A number.
    Number(i64),
}

/// A footnote token's facts (markdown-it's `meta`; what it does not say is `None`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Note {
    /// The footnote's number in the list, from 0.
    pub id: Option<usize>,
    /// Which reference to it this is, from 0.
    pub sub_id: Option<usize>,
    /// Its label, when written (`[^label]`).
    pub label: Option<String>,
}

/// One token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// What it is.
    pub kind: Kind,
    /// Its HTML tag (`p`, `h2`, `td`).
    pub tag: String,
    /// 1 opens, -1 closes, 0 neither.
    pub nesting: i8,
    /// Its nesting level.
    pub level: i32,
    /// Its text.
    pub content: String,
    /// The markup that made it (`**`, `\*`).
    pub markup: String,
    /// More about it (a fence's info string, `escape`, an item's number).
    pub info: String,
    /// Its attributes, in order.
    pub attrs: Vec<(String, Value)>,
    /// The source lines it covers, `[first, end)`.
    pub map: Option<(usize, usize)>,
    /// An inline token's children, or an image's alt text as tokens.
    pub children: Option<Vec<Token>>,
    /// A footnote token's facts.
    pub meta: Option<Note>,
    /// A block-level token.
    pub block: bool,
    /// Left out when rendered (a tight list's paragraphs).
    pub hidden: bool,
}

impl Token {
    /// A token of `kind` with `tag`, nothing else said.
    #[must_use]
    pub fn new(kind: Kind, tag: &str, nesting: i8) -> Token {
        Token {
            kind,
            tag: tag.to_owned(),
            nesting,
            level: 0,
            content: String::new(),
            markup: String::new(),
            info: String::new(),
            attrs: Vec::new(),
            map: None,
            children: None,
            meta: None,
            block: false,
            hidden: false,
        }
    }

    /// The attribute's value.
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<&Value> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Set an attribute (replacing one of the name).
    pub fn set_attr(&mut self, name: &str, value: Value) {
        match self.attrs.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = value,
            None => self.attrs.push((name.to_owned(), value)),
        }
    }
}

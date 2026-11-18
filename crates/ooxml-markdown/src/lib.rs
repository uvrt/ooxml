//! Markdown in (docs/architecture/README.md, section 3: "Markdown in and out"; phase P2):
//! the tokens markdown-it-py gives for the dialects docx-agent reads, token for token, so a
//! text written into a document means here what it means there.
//!
//! Owns:
//! - [`Token`]s, markdown-it's token stream;
//! - the inline parser ([`parse_inline`]): markdown-it's rule chain, the delimiter pairing
//!   of emphasis (CommonMark's rules, with markdown-it's "rule of 3"), its text joins;
//! - the block parser ([`parse`]): CommonMark's blocks with GFM tables and markdown-it's
//!   footnote plugin, reference definitions and footnotes collected in an [`env::Env`];
//! - link destinations normalised and validated as markdown-it does it (`mdurl`,
//!   punycode);
//! - [`styled`]: one paragraph's text with inline emphasis and named character-style
//!   spans, as docx-agent's `markdown/styled.py` reads `word_set_text`'s text.
//!
//! Must not: know Word or any document model, hold state between calls, or panic on any
//! input (it is fuzzed).
#![forbid(unsafe_code)]

mod block;
mod chars;
mod core;
pub mod env;
mod html;
mod inline;
mod link;
mod punycode;
#[rustfmt::skip]
mod punct;
pub mod styled;
mod token;
mod url;

pub use inline::Dialect;
pub use token::{Kind, Note, Token, Value};

/// The inline tokens of `text` in `dialect`, as markdown-it-py's `parseInline` gives an
/// inline token's children: the text normalised (line ends as `\n`, NUL as U+FFFD), the
/// inline rules run, escapes joined into the text around them.
#[must_use]
pub fn parse_inline(text: &str, dialect: Dialect) -> Vec<Token> {
    let source: Vec<char> = core::normalize(text).chars().collect();
    let tokens = inline::parse(&source, dialect, &mut env::Env::default());
    core::text_join(tokens)
}

/// The tokens of a Markdown document as markdown-it-py gives them for docx-agent's
/// dialect (CommonMark, GFM tables and strikethrough, footnotes), and what the read
/// collected (reference definitions, footnotes): the text normalised, its blocks read,
/// each block's inline content read into its children, footnotes moved to the end,
/// escapes joined into the text around them.
#[must_use]
pub fn parse(text: &str) -> (Vec<Token>, env::Env) {
    let source: Vec<char> = core::normalize(text).chars().collect();
    let mut env = env::Env::default();
    if source.is_empty() {
        return (Vec::new(), env);
    }
    let mut tokens = {
        let mut state = block::State::new(&source, &mut env, true);
        let end = state.line_max;
        block::tokenize(&mut state, 0, end);
        state.tokens
    };
    for token in tokens.iter_mut().filter(|t| t.kind == Kind::Inline) {
        let content: Vec<char> = token.content.chars().collect();
        token.children = Some(inline::parse(&content, Dialect::Document, &mut env));
    }
    let mut tokens = core::footnote_tail(tokens, &mut env);
    core::join_inline(&mut tokens);
    (tokens, env)
}

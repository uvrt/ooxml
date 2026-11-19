//! Owns: the definition of `word_design_facts` (docx-agent's `tools/style.py`), byte-equal
//! to today's: the fact kinds a call asks for and the cursor of `declared_styles`' pages.
//!
//! Must not: hold a handler.

use ooxml_tools::{ArgEnum, ToolArgs};

/// A kind of design fact, in the order the facts are reported.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DesignKind {
    /// Paragraph styles in use, per role.
    Styles,
    /// Table styles in use, by place.
    TableStyles,
    /// Top-level body tables' frames.
    TableFrames,
    /// Each section's page grid.
    Sections,
    /// The theme's colours and fonts.
    Theme,
    /// Repeated block shapes under headings.
    Patterns,
    /// Every declared style resolved (only when asked for).
    DeclaredStyles,
}

impl DesignKind {
    /// Every kind, in the order reported.
    pub const ALL: [DesignKind; 7] = [
        DesignKind::Styles,
        DesignKind::TableStyles,
        DesignKind::TableFrames,
        DesignKind::Sections,
        DesignKind::Theme,
        DesignKind::Patterns,
        DesignKind::DeclaredStyles,
    ];

    /// The kinds reported by default: all but `declared_styles`.
    pub const DEFAULT: [DesignKind; 6] = [
        DesignKind::Styles,
        DesignKind::TableStyles,
        DesignKind::TableFrames,
        DesignKind::Sections,
        DesignKind::Theme,
        DesignKind::Patterns,
    ];
}

/// `word_design_facts`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_design_facts",
    description = include_str!("word_design_facts.md"),
    group = "word_style"
)]
pub struct WordDesignFacts {
    /// Document id.
    pub doc: String,
    /// Facts to report. Default all but declared_styles.
    pub include: Option<Vec<DesignKind>>,
    /// `next_cursor` (`declared_styles`).
    #[arg(description = "next_cursor (declared_styles).")]
    pub cursor: Option<String>,
}

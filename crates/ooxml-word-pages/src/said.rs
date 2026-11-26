//! Owns: what a version laid out says ([`Said`]): its warnings ([`LayoutWarning`]), each
//! recorded once (`Layout.warn`'s rule: the same code, message and page); its
//! approximations ([`Approximation`]), each a warning too (`layout-approximate:<reason>`);
//! and the placeholders it drew where it carried on past what it cannot lay out
//! ([`DrawnPlaceholder`], `layout-placeholder:<reason>`, warned of too).
//!
//! Must not: find what to say ([`crate::warnings`] does).

/// One warning: docx2svg's `(code, message, 1-based page or None)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutWarning {
    /// Its stable code (`table-grid-mismatch`, `field-not-computed:PAGEREF`...).
    pub code: String,
    /// What it says.
    pub message: String,
    /// The 1-based page it is of, where it is of one.
    pub page: Option<usize>,
}

/// One approximation: docx2svg's `(code, message, 1-based page, element path)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approximation {
    /// `layout-approximate:<reason>`.
    pub code: String,
    /// What it says.
    pub message: String,
    /// The 1-based page it is on.
    pub page: Option<usize>,
    /// The element it is of.
    pub path: Option<String>,
}

/// One placeholder drawn where the layout carried on past what it cannot lay out:
/// docx2svg's `(code, message, 1-based page, element path)` (`Layout.placeholders`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawnPlaceholder {
    /// `layout-placeholder:<reason>`.
    pub code: String,
    /// What it says.
    pub message: String,
    /// The 1-based page it is drawn on.
    pub page: Option<usize>,
    /// The element it stands for.
    pub path: Option<String>,
    /// Its paragraph's key, beside its path (`None`: one no story's index holds).
    pub key: Option<ooxml_word_model::ids::BlockKey>,
}

impl DrawnPlaceholder {
    /// Its reason: its code after the colon.
    #[must_use]
    pub fn reason(&self) -> &str {
        self.code.split_once(':').map_or(self.code.as_str(), |(_, r)| r)
    }
}

/// The warnings, approximations and placeholders of a version, in docx2svg's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Said {
    /// Every warning, approximations' and placeholders' included.
    pub warnings: Vec<LayoutWarning>,
    /// The approximations.
    pub approximations: Vec<Approximation>,
    /// The placeholders drawn.
    pub placeholders: Vec<DrawnPlaceholder>,
}

impl Said {
    pub(crate) fn warn(&mut self, code: impl Into<String>, message: impl Into<String>, page: Option<usize>) {
        let entry = LayoutWarning {
            code: code.into(),
            message: message.into(),
            page,
        };
        if !self.warnings.contains(&entry) {
            self.warnings.push(entry);
        }
    }

    /// Take what `other` said, after what this said: each warning and approximation once.
    pub(crate) fn extend(&mut self, other: &Said) {
        for w in &other.warnings {
            self.warn(w.code.clone(), w.message.clone(), w.page);
        }
        for a in &other.approximations {
            if !self.approximations.contains(a) {
                self.approximations.push(a.clone());
            }
        }
        for p in &other.placeholders {
            if !self.placeholders.contains(p) {
                self.placeholders.push(p.clone());
            }
        }
    }

    /// Placeholder `placeholder` drawn on 1-based page `page` (`Layout.placeholder`): recorded
    /// once, and warned of.
    pub(crate) fn placeholder(
        &mut self,
        placeholder: &ooxml_word_layout::placeholder::Placeholder,
        page: Option<usize>,
    ) {
        let entry = DrawnPlaceholder {
            code: format!("layout-placeholder:{}", placeholder.reason),
            message: placeholder.drawn_message(None),
            page,
            path: Some(placeholder.path.to_string()),
            key: placeholder.key,
        };
        if !self.placeholders.contains(&entry) {
            self.placeholders.push(entry.clone());
        }
        self.warn(entry.code, entry.message, page);
    }

    pub(crate) fn approximate(&mut self, reason: &str, message: &str, page: Option<usize>, path: Option<&str>) {
        let entry = Approximation {
            code: format!("layout-approximate:{reason}"),
            message: message.to_owned(),
            page,
            path: path.map(str::to_owned),
        };
        if !self.approximations.contains(&entry) {
            self.approximations.push(entry.clone());
        }
        self.warn(entry.code, entry.message, page);
    }
}

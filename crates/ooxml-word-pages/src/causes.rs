//! Owns: why a version was laid out whole although a version before was laid out
//! ([`RelaidCause`], docx2svg's `Layout.relaid_cause` in its words): the kind of change a
//! part's name says, the styles' or sections' or stories' change found.
//!
//! Must not: lay anything out, or decide what is laid out whole (the layout does).

/// Why a version was laid out whole although a version before was laid out
/// (docx2svg's `Layout.relaid_cause`, in its words, `relayout.FALLBACKS`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelaidCause {
    /// docx2svg's reason: `styles`, `numbering`, `settings`, `theme`, `fonts`,
    /// `sections`, `headers-footers`.
    pub reason: &'static str,
    /// The body element it is at (`w:body/w:p[4]`), where it is one.
    pub at: Option<String>,
    /// That element's first paragraph's `w14:paraId`.
    pub para_id: Option<String>,
    /// The part that changed, where it is a part.
    pub part: Option<String>,
}

/// The kind of change a part's name says (`_PART_REASONS`).
pub(crate) fn part_reason(name: &str) -> Option<&'static str> {
    let story = |n: &str| {
        let n = n
            .strip_prefix("word/_rels/")
            .and_then(|n| n.strip_suffix(".rels"))
            .unwrap_or(n);
        let n = n.strip_prefix("word/").unwrap_or(n);
        ["header", "footer"].iter().any(|k| {
            n.strip_prefix(k)
                .and_then(|rest| rest.strip_suffix(".xml"))
                .is_some_and(|digits| digits.chars().all(|c| c.is_ascii_digit()))
        })
    };
    match name {
        "word/styles.xml" | "word/stylesWithEffects.xml" => Some("styles"),
        "word/numbering.xml" => Some("numbering"),
        "word/settings.xml" => Some("settings"),
        "word/fontTable.xml" => Some("fonts"),
        "word/footnotes.xml" | "word/endnotes.xml" => Some("notes"),
        n if n.starts_with("word/theme/") => Some("theme"),
        n if n.starts_with("word/fonts/") => Some("fonts"),
        n if story(n) => Some("headers-footers"),
        _ => None,
    }
}

/// The styles, numbering, settings or theme changed: the first such part, by name.
pub(crate) fn context_cause(changed: &[String]) -> RelaidCause {
    changed
        .iter()
        .find_map(|n| {
            part_reason(n)
                .filter(|r| matches!(*r, "styles" | "numbering" | "settings" | "theme"))
                .map(|reason| RelaidCause {
                    reason,
                    part: Some(n.clone()),
                    ..RelaidCause::default()
                })
        })
        .unwrap_or(RelaidCause {
            reason: "styles",
            ..RelaidCause::default()
        })
}

/// Why `laid` was laid out whole from `previous`: its sections changed, or its headers'
/// and footers' heights.
pub(crate) fn whole_cause(
    previous: &ooxml_word_layout::DocumentLayout,
    laid: &ooxml_word_layout::DocumentLayout,
    changed: &[String],
) -> Option<RelaidCause> {
    if previous.sections != laid.sections {
        let (old, new) = (&previous.document.sections, &laid.document.sections);
        let k = (0..new.len().max(old.len()))
            .find(|k| old.get(*k) != new.get(*k))
            .unwrap_or(0);
        let section = new.get(k).or_else(|| new.last());
        let mut cause = RelaidCause {
            reason: "sections",
            ..RelaidCause::default()
        };
        if let Some(section) = section {
            let top: Vec<&ooxml_word_model::model::Paragraph> = laid
                .document
                .body
                .iter()
                .filter_map(|b| match b {
                    ooxml_word_model::model::Block::Paragraph(p) => Some(&**p),
                    ooxml_word_model::model::Block::Table(_) => None,
                })
                .collect();
            if let Some(p) = section.last_paragraph.checked_sub(1).and_then(|k| top.get(k)) {
                let at: String = p.path.split('/').take(2).collect::<Vec<_>>().join("/");
                cause.at = Some(at);
                cause.para_id = p.para_id.as_deref().map(str::to_owned);
            }
        }
        return Some(cause);
    }
    if previous.stories != laid.stories {
        return Some(RelaidCause {
            reason: "headers-footers",
            part: changed
                .iter()
                .find(|n| part_reason(n) == Some("headers-footers"))
                .cloned(),
            ..RelaidCause::default()
        });
    }
    None
}

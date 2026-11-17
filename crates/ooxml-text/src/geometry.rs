//! Owns: where a paragraph's lines start and end and where its tabs go ([`Geometry`],
//! [`next_tab`]), and Word's compatibility rules for them as data ([`WordRules`]): the
//! justified squeeze of mode 15, the hyphenation zone and mode 15's own threshold, the
//! application's default tab stop and zone (docx2svg's `linebreak.Geometry`, `next_tab`,
//! `hyphen_rules`).
//!
//! Must not: read paragraph properties (a caller resolves indents, stops and settings).

use crate::units::{twips, Units};

/// A tab stop's alignment, ordered as their names compare (docx2svg takes the first stop
/// by position, then by alignment name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TabAlign {
    /// `center`.
    Center,
    /// `decimal`.
    Decimal,
    /// `end`.
    End,
    /// `left`.
    Left,
    /// `num`.
    Num,
    /// `right`.
    Right,
    /// `start`.
    Start,
}

impl TabAlign {
    /// The alignment of `w:tab/@w:val`; `None` for `clear`, `bar` and anything else, which
    /// are no stops for the text.
    pub fn parse(value: &str) -> Option<TabAlign> {
        Some(match value {
            "center" => TabAlign::Center,
            "decimal" => TabAlign::Decimal,
            "end" => TabAlign::End,
            "left" => TabAlign::Left,
            "num" => TabAlign::Num,
            "right" => TabAlign::Right,
            "start" => TabAlign::Start,
            _ => return None,
        })
    }

    /// Whether the text after the tab moves left from it (right, centre and decimal stops).
    pub fn pulls_back(self) -> bool {
        matches!(self, TabAlign::Right | TabAlign::Center | TabAlign::Decimal)
    }
}

/// Where a paragraph's lines start and end, from the column's left edge, in 1/4096 pt.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Geometry {
    /// The first line's start.
    pub first_start: Units,
    /// Every later line's start.
    pub start: Units,
    /// The right edge.
    pub right: Units,
    /// The custom tab stops: position and alignment.
    pub tabs: Vec<(Units, TabAlign)>,
    /// The default stops' interval.
    pub default_tab: Units,
    /// `w:ind/@w:left` when the paragraph hangs: a stop on its first line.
    pub hanging_stop: Option<Units>,
    /// Justified text in mode 15: a line may run past the edge by a quarter of each space.
    pub squeeze: bool,
    /// Where tab stops are measured from.
    pub tab_origin: Units,
    /// Below mode 15: the hyphenation zone.
    pub hyphen_zone: Option<Units>,
    /// Below mode 15: the letter after an automatic hyphen must fit too.
    pub hyphen_next: bool,
    /// Mode 15, not justified: the room after the line's text must exceed this.
    pub hyphen_gap: Option<Units>,
    /// `w:consecutiveHyphenLimit` (0: none).
    pub hyphen_limit: u32,
}

/// The stop a tab at `x` jumps to, its alignment, and whether it is a default stop.
pub fn next_tab(x: Units, geometry: &Geometry, first_line: bool) -> (Units, TabAlign, bool) {
    let origin = geometry.tab_origin;
    let mut best: Option<(Units, TabAlign)> = None;
    let mut consider = |candidate: (Units, TabAlign)| {
        if best.is_none_or(|b| candidate < b) {
            best = Some(candidate);
        }
    };
    for &(position, align) in &geometry.tabs {
        if position + origin > x {
            consider((position + origin, align));
        }
    }
    if first_line {
        if let Some(stop) = geometry.hanging_stop.filter(|s| *s > x) {
            consider((stop, TabAlign::Left));
        }
    }
    if let Some((stop, align)) = best {
        return (stop, align, false);
    }
    let last = geometry.tabs.iter().map(|t| t.0).max().unwrap_or(Units::ZERO) + origin;
    let base = if x > last { x } else { last };
    let step = geometry.default_tab;
    let steps = (base - origin).checked_div(step).map_or(0, Units::floor) + 1;
    let stop = step.scale(steps, 1).unwrap_or(step) + origin;
    (stop, TabAlign::Left, true)
}

/// Word's compatibility rules for lines, as data (docx2svg's measured constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WordRules {
    /// The document's compatibility mode (`w:compatSetting compatibilityMode`; 0 unstated).
    pub compatibility_mode: u32,
}

/// Word's tab interval when `w:defaultTabStop` is unstated: 708 twips (1.25 cm), this
/// machine's metric `Normal.dotm`.
pub const APPLICATION_DEFAULT_TAB: i64 = 708;
/// The hyphenation zone when unstated: 425 twips (0.75 cm), the application's default.
pub const APPLICATION_DEFAULT_ZONE: i64 = 425;
/// Mode 15's threshold in text not justified: 351.5 twips, as (numerator, denominator).
pub const MODE_15_HYPHEN_GAP: (i128, i128) = (3515, 10);

/// The hyphenation fields of a [`Geometry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HyphenFields {
    /// [`Geometry::hyphen_zone`].
    pub zone: Option<Units>,
    /// [`Geometry::hyphen_next`].
    pub next: bool,
    /// [`Geometry::hyphen_gap`].
    pub gap: Option<Units>,
    /// [`Geometry::hyphen_limit`].
    pub limit: u32,
}

impl WordRules {
    /// Whether the mode is 15 or later.
    pub fn mode15(self) -> bool {
        self.compatibility_mode >= 15
    }

    /// Whether a paragraph's lines squeeze their spaces: justified (`both`) in mode 15.
    pub fn squeeze(self, justified_both: bool) -> bool {
        self.mode15() && justified_both
    }

    /// The default stops' interval for `w:defaultTabStop` (twips; unstated or not above
    /// zero: the application's).
    pub fn default_tab(self, default_tab_stop: Option<i64>) -> Units {
        twips(default_tab_stop.filter(|v| *v > 0).unwrap_or(APPLICATION_DEFAULT_TAB))
    }

    /// The hyphenation fields of a paragraph of a hyphenated document: `zone` twips
    /// (`None`: unstated), `limit` lines, `justified` (`both` or `distribute`).
    pub fn hyphenation(self, zone: Option<i64>, limit: u32, justified: bool) -> HyphenFields {
        if self.mode15() {
            let (n, d) = MODE_15_HYPHEN_GAP;
            HyphenFields {
                gap: (!justified).then(|| Units::ratio(n * 4096, d * 20)).flatten(),
                limit,
                ..HyphenFields::default()
            }
        } else {
            HyphenFields {
                zone: Some(twips(zone.unwrap_or(APPLICATION_DEFAULT_ZONE).max(0))),
                next: true,
                gap: None,
                limit,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(tabs: Vec<(Units, TabAlign)>) -> Geometry {
        Geometry {
            first_start: Units::ZERO,
            start: Units::ZERO,
            right: Units::int(100_000),
            tabs,
            default_tab: Units::int(1000),
            hanging_stop: Some(Units::int(500)),
            squeeze: false,
            tab_origin: Units::ZERO,
            hyphen_zone: None,
            hyphen_next: false,
            hyphen_gap: None,
            hyphen_limit: 0,
        }
    }

    #[test]
    fn stops() {
        let g = geometry(vec![
            (Units::int(700), TabAlign::Right),
            (Units::int(700), TabAlign::Center),
        ]);
        assert_eq!(
            next_tab(Units::int(100), &g, true),
            (Units::int(500), TabAlign::Left, false)
        );
        assert_eq!(
            next_tab(Units::int(600), &g, true),
            (Units::int(700), TabAlign::Center, false)
        );
        assert_eq!(
            next_tab(Units::int(700), &g, false),
            (Units::int(1000), TabAlign::Left, true)
        );
        assert_eq!(
            next_tab(Units::int(2500), &g, false),
            (Units::int(3000), TabAlign::Left, true)
        );
        let rules = WordRules { compatibility_mode: 15 };
        assert_eq!(rules.hyphenation(None, 0, false).gap, Units::ratio(3515 * 4096, 200));
        assert_eq!(
            WordRules { compatibility_mode: 14 }.hyphenation(None, 2, false).zone,
            Some(twips(425))
        );
    }
}

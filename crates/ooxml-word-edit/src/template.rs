//! Owns: [`UpgradeToModern`], Word's File > Info > Convert as docx-agent's
//! `upgrade_to_modern` writes it (`edit/authoring.py`, `upgrade`; measured on modes 11, 12
//! and 14 with every legacy option set): only the settings' `w:compat` changes --
//! `compatibilityMode` becomes 15 with the five settings Word writes beside it, the ten
//! legacy options Word 2013's layout still honours stay (and `w:useFELayout`, which a save
//! drops, not Convert), the other legacy options go -- and [`compatibility_mode`], the mode
//! a document is in; and [`UpgradeReflowed`], Convert with the reflow it makes, measured
//! in the edit's transaction so that a stop laying out changes nothing.
//!
//! Must not: lay out (the reflow the tool reports is the host's layout's, through the
//! layout port), or touch any part but the settings.

use crate::doc::{Key, WordDoc};
use crate::fields::{EditLayout, PageReflow};
use crate::names::W;
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::review::xml::{attr, insert_in_order, make, set_attr};
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_store::{Command, Txn};
use ooxml_word_model::parts::{related, REL_SETTINGS};
use std::collections::HashMap;

/// The mode a document that names none is in (Word 2007's).
pub const DEFAULT_COMPATIBILITY_MODE: i64 = 12;

/// The mode Convert brings a document into.
pub const MODERN_MODE: i64 = 15;

/// The `w:compatSetting`s' `w:uri` Word writes.
const COMPAT_URI: &str = "http://schemas.microsoft.com/office/word";

/// Every child of `w:compat` but `w:compatSetting`, in schema order (ECMA-376 `CT_Compat`).
const COMPAT_OPTIONS: [&str; 65] = [
    "useSingleBorderforContiguousCells",
    "wpJustification",
    "noTabHangInd",
    "noLeading",
    "spaceForUL",
    "noColumnBalance",
    "balanceSingleByteDoubleByteWidth",
    "noExtraLineSpacing",
    "doNotLeaveBackslashAlone",
    "ulTrailSpace",
    "doNotExpandShiftReturn",
    "spacingInWholePoints",
    "lineWrapLikeWord6",
    "printBodyTextBeforeHeader",
    "printColBlack",
    "wpSpaceWidth",
    "showBreaksInFrames",
    "subFontBySize",
    "suppressBottomSpacing",
    "suppressTopSpacing",
    "suppressSpacingAtTopOfPage",
    "suppressTopSpacingWP",
    "suppressSpBfAfterPgBrk",
    "swapBordersFacingPages",
    "convMailMergeEsc",
    "truncateFontHeightsLikeWP6",
    "mwSmallCaps",
    "usePrinterMetrics",
    "doNotSuppressParagraphBorders",
    "wrapTrailSpaces",
    "footnoteLayoutLikeWW8",
    "shapeLayoutLikeWW8",
    "alignTablesRowByRow",
    "forgetLastTabAlignment",
    "adjustLineHeightInTable",
    "autoSpaceLikeWord95",
    "noSpaceRaiseLower",
    "doNotUseHTMLParagraphAutoSpacing",
    "layoutRawTableWidth",
    "layoutTableRowsApart",
    "useWord97LineBreakRules",
    "doNotBreakWrappedTables",
    "doNotSnapToGridInCell",
    "selectFldWithFirstOrLastChar",
    "applyBreakingRules",
    "doNotWrapTextWithPunct",
    "doNotUseEastAsianBreakRules",
    "useWord2002TableStyleRules",
    "growAutofit",
    "useFELayout",
    "useNormalStyleForList",
    "doNotUseIndentAsNumberingTabStop",
    "useAltKinsokuLineBreakRules",
    "allowSpaceOfSameStyleInTable",
    "doNotSuppressIndentation",
    "doNotAutofitConstrainedTables",
    "autofitToFirstFixedWidthCell",
    "underlineTabInNumList",
    "displayHangulFixedWidth",
    "splitPgBreakAndParaMark",
    "doNotVertAlignCellWithSp",
    "doNotBreakConstrainedForcedTable",
    "doNotVertAlignInTxbx",
    "useAnsiKerningPairs",
    "cachedColBalance",
];

/// The legacy options Convert keeps (measured: these ten of the 65, in every mode), and
/// `useFELayout`, which a save drops, not Convert.
const KEPT: [&str; 11] = [
    "spaceForUL",
    "balanceSingleByteDoubleByteWidth",
    "noExtraLineSpacing",
    "doNotLeaveBackslashAlone",
    "ulTrailSpace",
    "doNotExpandShiftReturn",
    "suppressBottomSpacing",
    "adjustLineHeightInTable",
    "doNotUseHTMLParagraphAutoSpacing",
    "applyBreakingRules",
    "useFELayout",
];

/// The `w:compatSetting`s Convert writes, in Word's order, and their values.
const MODERN_SETTINGS: [(&str, &str); 6] = [
    ("compatibilityMode", "15"),
    ("overrideTableStyleFontSizeAndJustification", "1"),
    ("enableOpenTypeFeatures", "1"),
    ("doNotFlipMirrorIndents", "1"),
    ("differentiateMultirowTableHeaders", "1"),
    ("useWord2013TrackBottomHyphenation", "0"),
];

/// The settings part of `doc`, if it has one.
fn settings_of(doc: &WordDoc) -> Result<Option<String>> {
    let docx = doc.docx()?;
    Ok(related(doc.package(), &docx.main, REL_SETTINGS)?)
}

/// The compatibility mode `doc` is in: the settings' `w:compat`'s first
/// `w:compatSetting w:name="compatibilityMode"`, or 12 when it names none or not a number
/// (docx-agent's `compatibility_mode`).
///
/// # Errors
/// When the settings part is not XML.
pub fn compatibility_mode(doc: &WordDoc) -> Result<i64> {
    let Some(part) = settings_of(doc)? else {
        return Ok(DEFAULT_COMPATIBILITY_MODE);
    };
    let Some(tree) = doc.tree(&part)? else {
        return Ok(DEFAULT_COMPATIBILITY_MODE);
    };
    let Some(compat) = tree.root().find(Some(W), "compat") else {
        return Ok(DEFAULT_COMPATIBILITY_MODE);
    };
    let setting = compat
        .child_elements()
        .filter(|e| e.name().is(Some(W), "compatSetting"))
        .find(|e| e.attribute(Some(W), "name") == Some("compatibilityMode"));
    Ok(setting
        .and_then(|e| crate::validate::python_int(e.attribute(Some(W), "val").unwrap_or("")))
        .unwrap_or(DEFAULT_COMPATIBILITY_MODE))
}

/// Bring the document into compatibility mode 15 as Word's Convert does; a document
/// already in it is left as it is. Never tracked (docx-agent's `track=False`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpgradeToModern;

/// What Convert did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Upgraded {
    /// Whether the document changed (not when it already was in mode 15).
    pub changed: bool,
    /// The legacy options removed, by name, in the order they were written.
    pub removed: Vec<String>,
}

impl Command<WordDoc> for UpgradeToModern {
    type Out = Upgraded;
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Upgraded> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        if let Some(part) = settings_of(sink.doc())? {
            sink.read(Key::part(part.as_str()));
        }
        if compatibility_mode(sink.doc())? >= MODERN_MODE {
            return Ok(Upgraded::default());
        }
        let part = crate::styles::settings_part(&mut sink)?;
        let held = sink.doc().clone();
        let orders = held.shared().facts.orders();
        let mut space = Space::default();
        let work = space
            .work(&held, &part)?
            .ok_or_else(|| EditError::no_part(part.clone()))?;
        let root = work.root();
        let compat = match work.find(root, W, "compat") {
            Some(found) => found,
            None => {
                let made = make(work, "compat", &[]);
                insert_in_order(work, root, made, orders);
                made
            }
        };
        let removed = upgrade(work, compat);
        space.mark_dirty(&part);
        space.finish(&mut sink)?;
        Ok(Upgraded { changed: true, removed })
    }
}

/// [`UpgradeToModern`] with the reflow it makes: the pages of the version before against
/// the version Convert made, laid out by the call's layout (docx-agent's `upgrade_to_modern`
/// lays out before and after). Measured in the transaction, so a stop laying out refuses the
/// edit and nothing changes; the reflow reads the whole document, so another writer's
/// commit makes it again on the newer version.
#[derive(Debug, Clone, Copy)]
pub struct UpgradeReflowed<'l> {
    /// The call's layout (`None`: no reflow is measured).
    pub layout: Option<EditLayout<'l>>,
}

/// What Convert did, and its reflow.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reflowed {
    /// What Convert did.
    pub upgraded: Upgraded,
    /// The reflow, where the document changed and a layout measured it; inside, why the
    /// layout cannot say.
    pub reflow: Option<std::result::Result<PageReflow, String>>,
}

impl Command<WordDoc> for UpgradeReflowed<'_> {
    type Out = Reflowed;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        UpgradeToModern.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Reflowed> {
        let upgraded = UpgradeToModern.prepare(tx)?;
        let Some(layout) = self.layout.filter(|_| upgraded.changed) else {
            return Ok(Reflowed { upgraded, reflow: None });
        };
        tx.read_everything();
        let reflow = layout.reflow(tx.base().doc(), tx.doc())?;
        Ok(Reflowed {
            upgraded,
            reflow: Some(reflow),
        })
    }
}

/// The children of `w:compat` sorted as Convert leaves them.
#[derive(Default)]
struct Sorted {
    /// The legacy options kept, by name (a later one of a name winning).
    options: HashMap<&'static str, Id>,
    /// The `w:compatSetting`s, in order.
    settings: Vec<Id>,
    /// Everything else (comments, other elements), in order.
    others: Vec<Id>,
    /// The legacy options removed, by name.
    removed: Vec<String>,
    /// The children not put back: removed options, and kept ones a later one of the same
    /// name replaced.
    dropped: Vec<Id>,
}

/// Sort `compat`'s children as docx-agent's `upgrade` does.
fn sort(work: &Work, compat: Id) -> Sorted {
    let mut out = Sorted::default();
    for &child in work.children(compat) {
        if !work.is_element(child) {
            out.others.push(child);
            continue;
        }
        if work.is(child, W, "compatSetting") {
            out.settings.push(child);
            continue;
        }
        let option = COMPAT_OPTIONS.iter().find(|name| work.is(child, W, name)).copied();
        match option {
            Some(name) if KEPT.contains(&name) => {
                if let Some(earlier) = out.options.insert(name, child) {
                    out.dropped.push(earlier);
                }
            }
            Some(name) => {
                out.removed.push(name.to_owned());
                out.dropped.push(child);
            }
            None => out.others.push(child),
        }
    }
    out
}

/// Convert's change to `compat`, in place: the legacy options it removes, by name.
fn upgrade(work: &mut Work, compat: Id) -> Vec<String> {
    let sorted = sort(work, compat);
    for &gone in &sorted.dropped {
        // lxml's `remove`: the element goes with its tail
        work.set_tail(gone, "");
        work.remove(gone);
    }
    let mut order: Vec<Id> = COMPAT_OPTIONS
        .iter()
        .filter_map(|name| sorted.options.get(name).copied())
        .collect();
    let mut placed = Vec::new();
    for (name, value) in MODERN_SETTINGS {
        // a later one of a name wins (docx-agent's `by_name`)
        let found = sorted.settings.iter().rev().copied().find(|&s| {
            attr(work, s, "name") == Some(name) && work.get(s, Some(W), "uri").is_none_or(|uri| uri == COMPAT_URI)
        });
        let node = match found {
            Some(node) => node,
            None => make(work, "compatSetting", &[("name", name), ("uri", COMPAT_URI)]),
        };
        set_attr(work, node, "val", value);
        placed.push(node);
    }
    order.extend(placed.iter().copied());
    order.extend(sorted.settings.iter().copied().filter(|s| !placed.contains(s)));
    order.extend(sorted.others.iter().copied());
    for node in order {
        work.append(compat, node);
    }
    sorted.removed
}

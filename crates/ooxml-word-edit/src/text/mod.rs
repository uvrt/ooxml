//! Owns: a paragraph's text as docx-agent defines it (`edit/text.py`), read from an
//! editable copy: its characters in a view of its revisions, each with where it comes from
//! ([`Atom`]), and its runs; the diff set_text writes by ([`diff`]); untracked writing
//! ([`write`]).
//!
//! The text: `w:t` characters; `w:tab` and `w:ptab` are a tab; `w:br`/`w:cr` a line break
//! (`\v`), a page or column break `\f`; `w:noBreakHyphen` U+2011; `w:softHyphen` U+00AD;
//! `w:sym` its character; a field its result; a drawing, a note reference, an equation or
//! another object U+FFFC.
//!
//! Must not: read anything outside the paragraph, or change it (writing is [`write`]'s
//! and the revision rules').

pub(crate) mod diff;
pub(crate) mod write;

use crate::names::{M, MC, W};
use crate::work::{Id, Work};

/// A line break as text.
pub(crate) const LINE_BREAK: char = '\u{b}';
/// A page or column break as text.
pub(crate) const PAGE_BREAK: char = '\u{c}';
/// An object as text.
pub(crate) const OBJECT: char = '\u{fffc}';
/// A non-breaking hyphen.
pub(crate) const NON_BREAKING_HYPHEN: char = '\u{2011}';
/// A soft hyphen.
pub(crate) const SOFT_HYPHEN: char = '\u{ad}';

/// Where a character comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtomKind {
    /// A character of a `w:t`.
    Text,
    /// The whole of an element (a tab, a break, an object).
    Element,
}

/// One character of a paragraph's text and where it comes from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Atom {
    /// The character.
    pub(crate) ch: char,
    /// Its kind.
    pub(crate) kind: AtomKind,
    /// The node it is in (or is).
    pub(crate) node: Id,
    /// Its place in the node's text, for text and deleted characters.
    pub(crate) offset: usize,
    /// The run it is in (`None` for an equation).
    pub(crate) run: Option<Id>,
    /// In a field's result.
    pub(crate) in_field: bool,
    /// The innermost field it is in the result of: its begin `w:fldChar`, or its
    /// `w:fldSimple`.
    pub(crate) field: Option<Id>,
}

impl Atom {
    /// A character of plain text: of a `w:t`, not in a field's result.
    pub(crate) fn plain(&self) -> bool {
        self.kind == AtomKind::Text && !self.in_field
    }
}

/// What a walk reads: the characters and the runs.
pub(crate) struct Walk {
    pub(crate) atoms: Vec<Atom>,
    pub(crate) runs: Vec<Id>,
}

struct State {
    atoms: Vec<Atom>,
    runs: Vec<Id>,
    /// Open complex fields, innermost last: whether each is in its result.
    fields: Vec<bool>,
    simple_field: usize,
    owners: Vec<Id>,
}

impl State {
    fn visible(&self) -> bool {
        self.fields.iter().all(|result| *result)
    }

    fn in_field(&self) -> bool {
        !self.fields.is_empty() || self.simple_field > 0
    }

    fn owner(&self) -> Option<Id> {
        self.owners.last().copied()
    }
}

/// The paragraph's characters and runs in document order, in the current view
/// (insertions in, deletions out; the original and markup views are the reading tools',
/// `ooxml-word-model`).
pub(crate) fn walk(work: &Work, paragraph: Id) -> Walk {
    let mut state = State {
        atoms: Vec::new(),
        runs: Vec::new(),
        fields: Vec::new(),
        simple_field: 0,
        owners: Vec::new(),
    };
    container(work, paragraph, &mut state);
    Walk {
        atoms: state.atoms,
        runs: state.runs,
    }
}

/// The paragraph's characters in the current view.
pub(crate) fn atoms(work: &Work, paragraph: Id) -> Vec<Atom> {
    walk(work, paragraph).atoms
}

/// The paragraph's text in the current view.
pub(crate) fn paragraph_text(work: &Work, paragraph: Id) -> String {
    atoms(work, paragraph).iter().map(|a| a.ch).collect()
}

/// The paragraph's runs in the current view.
pub(crate) fn runs(work: &Work, paragraph: Id) -> Vec<Id> {
    walk(work, paragraph).runs
}

const TRANSPARENT: &[&str] = &["hyperlink", "smartTag", "customXml", "dir", "bdo"];

/// Whether a revision container's content is in the current view; `None` for another
/// element.
fn shown(local: &str) -> Option<bool> {
    match local {
        "ins" | "moveTo" => Some(true),
        "del" | "moveFrom" => Some(false),
        _ => None,
    }
}

fn container(work: &Work, node: Id, state: &mut State) {
    for &child in work.children(node) {
        let Some(name) = work.name(child) else { continue };
        let in_w = name.namespace() == Some(W);
        let local = name.local();
        if in_w && local == "r" {
            run(work, child, state);
        } else if in_w && TRANSPARENT.contains(&local) {
            container(work, child, state);
        } else if in_w && local == "fldSimple" {
            state.simple_field += 1;
            state.owners.push(child);
            container(work, child, state);
            state.owners.pop();
            state.simple_field -= 1;
        } else if in_w && local == "sdt" {
            if let Some(content) = work.find(child, W, "sdtContent") {
                container(work, content, state);
            }
        } else if let Some(shown) = in_w.then(|| shown(local)).flatten() {
            if shown {
                container(work, child, state);
            }
        } else if name.namespace() == Some(M) && (local == "oMath" || local == "oMathPara") && state.visible() {
            let atom = Atom {
                ch: OBJECT,
                kind: AtomKind::Element,
                node: child,
                offset: 0,
                run: None,
                in_field: state.in_field(),
                field: state.owner(),
            };
            state.atoms.push(atom);
        }
    }
}

/// The character a run's child element is, if it is one.
fn character_of(work: &Work, child: Id, local: &str, namespace: Option<&str>) -> Option<char> {
    if namespace == Some(MC) {
        return (local == "AlternateContent").then_some(OBJECT);
    }
    if namespace != Some(W) {
        return None;
    }
    Some(match local {
        "tab" | "ptab" => '\t',
        "cr" => LINE_BREAK,
        "noBreakHyphen" => NON_BREAKING_HYPHEN,
        "softHyphen" => SOFT_HYPHEN,
        "br" => match work.get(child, Some(W), "type") {
            Some("page" | "column") => PAGE_BREAK,
            _ => LINE_BREAK,
        },
        "sym" => symbol(work, child),
        "drawing" | "pict" | "object" | "footnoteReference" | "endnoteReference" | "footnoteRef" | "endnoteRef"
        | "contentPart" | "ruby" => OBJECT,
        _ => return None,
    })
}

fn run(work: &Work, r: Id, state: &mut State) {
    state.runs.push(r);
    for &child in work.children(r) {
        let Some(name) = work.name(child) else { continue };
        let (namespace, local) = (name.namespace(), name.local());
        if namespace == Some(W) && local == "rPr" {
            continue;
        }
        if namespace == Some(W) && local == "fldChar" {
            match work.get(child, Some(W), "fldCharType") {
                Some("begin") => {
                    state.fields.push(false);
                    state.owners.push(child);
                }
                Some("separate") => {
                    if let Some(last) = state.fields.last_mut() {
                        *last = true;
                    }
                }
                Some("end") if !state.fields.is_empty() => {
                    state.fields.pop();
                    state.owners.pop();
                }
                _ => {}
            }
            continue;
        }
        if !state.visible() {
            continue;
        }
        let (in_field, field) = (state.in_field(), state.owner());
        // deleted text (`w:delText`) is not in the current view
        if namespace == Some(W) && local == "t" {
            for (offset, ch) in work.text(child).chars().enumerate() {
                state.atoms.push(Atom {
                    ch,
                    kind: AtomKind::Text,
                    node: child,
                    offset,
                    run: Some(r),
                    in_field,
                    field,
                });
            }
        } else if let Some(ch) = character_of(work, child, local, namespace) {
            state.atoms.push(Atom {
                ch,
                kind: AtomKind::Element,
                node: child,
                offset: 0,
                run: Some(r),
                in_field,
                field,
            });
        }
    }
}

fn symbol(work: &Work, node: Id) -> char {
    work.get(node, Some(W), "char")
        .and_then(|raw| u32::from_str_radix(raw, 16).ok())
        .and_then(char::from_u32)
        .unwrap_or(OBJECT)
}

//! Owns: everything the layout reads of the settings part ([`Settings`]), as docx2svg's
//! `settings_read` reads it: the theme's font languages, the compatibility mode, options
//! and settings, the default tab stop, even and odd headers, hyphenation, the colour
//! mapping, and the notes' settings.
//!
//! Must not: decide what an unstated setting means for layout (measured per rule there).

use crate::theme::colour_mapping;
use crate::value::{text, Text};
use crate::w;
use crate::xmlread::{attr_int, attr_on, children, elements, local};
use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet};

/// `w:autoHyphenation` is on, and what else the settings say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hyphenation {
    /// `w:hyphenationZone`, twips.
    pub zone: Option<i64>,
    /// `w:consecutiveHyphenLimit` (0: none).
    pub limit: i64,
    /// `w:doNotHyphenateCaps`.
    pub no_caps: bool,
}

/// The settings the layout reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// `w:themeFontLang`'s `val`, `eastAsia` and `bidi`, where stated and not empty.
    pub theme_font_lang: BTreeMap<Text, Text>,
    /// `w:compatSetting[@w:name="compatibilityMode"]`.
    pub compatibility_mode: Option<i64>,
    /// The legacy `w:compat` options that are on, by local name.
    pub compat_options: BTreeSet<Text>,
    /// The other `w:compatSetting`s, name -> `w:val` as written.
    pub compat_settings: BTreeMap<Text, Text>,
    /// `w:defaultTabStop`, twips.
    pub default_tab_stop: Option<i64>,
    /// `w:evenAndOddHeaders`.
    pub even_and_odd_headers: bool,
    /// Automatic hyphenation, `None` when off.
    pub hyphenation: Option<Hyphenation>,
    /// `w:clrSchemeMapping`.
    pub color_map: BTreeMap<Text, Text>,
    /// `w:endnotePr/w:pos`.
    pub endnote_position: Option<Text>,
    /// The notes `w:endnotePr` names (`w:endnote/@w:id`).
    pub endnote_named: BTreeSet<i64>,
    /// The notes `w:footnotePr` names.
    pub footnote_named: BTreeSet<i64>,
}

fn named(element: Option<&Element>, kind: &str) -> BTreeSet<i64> {
    element
        .into_iter()
        .flat_map(|e| children(e, kind))
        .filter_map(|n| attr_int(n, "id"))
        .collect()
}

fn hyphenation(root: w::Settings<'_>) -> Option<Hyphenation> {
    let auto = root.auto_hyphenation()?;
    if !attr_on(auto.element(), "val") {
        return None;
    }
    Some(Hyphenation {
        zone: root.hyphenation_zone().and_then(w::Val::twips),
        limit: root.hyphen_limit().and_then(w::Val::int).unwrap_or(0),
        no_caps: root.no_caps_hyphenation().is_some_and(|c| attr_on(c.element(), "val")),
    })
}

fn compat(root: w::Settings<'_>, settings: &mut Settings) {
    let Some(compat) = root.compat() else { return };
    let mut mode_found = false;
    for option in elements(compat.element()) {
        if local(option) != "compatSetting" {
            if attr_on(option, "val") {
                settings.compat_options.insert(text(local(option)));
            }
            continue;
        }
        let setting = w::CompatSetting::new(option);
        match setting.name() {
            Some("compatibilityMode") => {
                if !mode_found {
                    settings.compatibility_mode = attr_int(option, "val");
                    mode_found = true;
                }
            }
            Some(name) if !name.is_empty() => {
                settings
                    .compat_settings
                    .insert(text(name), text(setting.val().unwrap_or("")));
            }
            _ => {}
        }
    }
}

impl Settings {
    /// The settings part.
    pub fn read(root: &Element) -> Settings {
        let view = w::Settings::new(root);
        let mut settings = Settings::default();
        if let Some(lang) = view.theme_font_lang() {
            for (k, v) in [
                ("val", lang.val()),
                ("eastAsia", lang.east_asia()),
                ("bidi", lang.bidi()),
            ] {
                if let Some(v) = v.filter(|v| !v.is_empty()) {
                    settings.theme_font_lang.insert(text(k), text(v));
                }
            }
        }
        compat(view, &mut settings);
        settings.default_tab_stop = view.default_tab_stop().and_then(w::Val::twips);
        settings.even_and_odd_headers = view.even_and_odd_headers().is_some_and(|e| attr_on(e.element(), "val"));
        settings.hyphenation = hyphenation(view);
        settings.color_map = colour_mapping(root);
        let endnotes = view.endnotes();
        settings.endnote_position = endnotes.and_then(w::NotePr::position).and_then(w::Val::val).map(text);
        settings.endnote_named = named(endnotes.map(w::NotePr::element), "endnote");
        settings.footnote_named = named(view.footnotes().map(w::NotePr::element), "footnote");
        settings
    }

    /// Whether the legacy option `name` is on.
    pub fn option(&self, name: &str) -> bool {
        self.compat_options.contains(name)
    }

    /// A compatibility setting's value as written.
    pub fn compat_setting(&self, name: &str) -> Option<&str> {
        self.compat_settings.get(name).map(|v| &**v)
    }
}

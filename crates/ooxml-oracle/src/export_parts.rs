//! The parts of the Word model's export besides the body: sections, stories, notes, styles,
//! numbering, settings, the theme, picture bullets, cached fields and warnings.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::export::{blocks, props, section_props};
use ooxml_word_model::document::Notes;
use ooxml_word_model::docx::VersionNames;
use ooxml_word_model::numbering::Level;
use ooxml_word_model::styles::Style;
use ooxml_word_model::theme::ThemeFonts;
use ooxml_word_model::value::Text;
use ooxml_word_model::WordDocument;
use serde_json::{json, Map, Value as J};
use std::collections::{BTreeMap, BTreeSet};

fn texts<'a>(items: impl Iterator<Item = &'a Text>) -> J {
    J::Array(items.map(|t| json!(&**t)).collect())
}

fn text_map(m: &BTreeMap<Text, Text>) -> J {
    J::Object(m.iter().map(|(k, v)| (k.to_string(), json!(&**v))).collect())
}

fn ints(s: &BTreeSet<i64>) -> J {
    json!(s.iter().collect::<Vec<_>>())
}

fn style(s: &Style) -> J {
    let conditional: Map<String, J> = s
        .conditional
        .iter()
        .map(|(k, r)| {
            (
                k.to_string(),
                json!([
                    props(&r.paragraph),
                    props(&r.run),
                    props(&r.table),
                    props(&r.row),
                    props(&r.cell)
                ]),
            )
        })
        .collect();
    json!({
        "style_id": &*s.style_id, "kind": &*s.kind_name, "name": s.name.as_deref(), "based_on": s.based_on.as_deref(),
        "next_style": s.next_style.as_deref(), "link": s.link.as_deref(), "default": s.default,
        "paragraph": props(&s.paragraph), "run": props(&s.run), "conditional": conditional,
        "table": props(&s.table), "table_row": props(&s.table_row), "table_cell": props(&s.table_cell),
    })
}

fn level(l: &Level) -> J {
    json!({
        "level": l.level, "format": l.format.as_deref(), "text": l.text.as_deref(), "paragraph": props(&l.paragraph),
        "run": props(&l.run), "suffix": l.suffix.as_deref(), "start": l.start, "restart": l.restart, "legal": l.legal,
        "alignment": l.alignment.as_deref(), "picture_bullet": l.picture_bullet, "style": l.style.as_deref(),
        "single": l.single,
    })
}

fn fonts(f: &ThemeFonts) -> J {
    json!({"latin": &*f.latin, "east_asian": &*f.east_asian, "complex_script": &*f.complex_script, "scripts": text_map(&f.scripts)})
}

fn notes(n: &Notes, names: &VersionNames) -> (J, J, J) {
    let by_id = J::Object(n.notes.iter().map(|(k, v)| (k.to_string(), blocks(v, names))).collect());
    let separators = J::Object(
        n.separators
            .iter()
            .map(|(k, v)| (k.to_string(), blocks(v, names)))
            .collect(),
    );
    let ids = J::Object(n.separator_ids.iter().map(|(k, v)| (k.to_string(), json!(v))).collect());
    (by_id, separators, ids)
}

/// Everything of the export but the body and what is resolved.
pub fn parts(d: &WordDocument) -> J {
    let cx = &d.context;
    let s = &cx.settings;
    let numbering = &cx.numbering;
    let (footnotes, footnote_separators, footnote_separator_ids) = notes(&d.footnotes, &d.names);
    let (endnotes, endnote_separators, endnote_separator_ids) = notes(&d.endnotes, &d.names);
    json!({
        "sections": J::Array(d.sections.iter().map(|x| {
            let mut out = section_props(&x.properties);
            if let Some(o) = out.as_object_mut() {
                o.insert("first_paragraph".into(), json!(x.first_paragraph));
                o.insert("last_paragraph".into(), json!(x.last_paragraph));
            }
            out
        }).collect()),
        "styles": cx.styles.as_ref().map(|sheet| json!({
            "run_defaults": props(&sheet.run_defaults), "paragraph_defaults": props(&sheet.paragraph_defaults),
            "styles": J::Object(sheet.styles.iter().map(|(k, v)| (k.to_string(), style(v))).collect()),
        })),
        "font_scheme": cx.font_scheme.as_ref().map(|f| json!({"major": fonts(&f.major), "minor": fonts(&f.minor)})),
        "theme_colors": text_map(&cx.colours.scheme),
        "color_map": text_map(&s.color_map),
        "numbering": J::Object(numbering.levels.iter().map(|(n, levels)| {
            (n.to_string(), J::Object(levels.iter().map(|(k, l)| (k.to_string(), level(l))).collect()))
        }).collect()),
        "numbering_lists": J::Object(numbering.lists.iter().map(|(k, v)| (k.to_string(), json!(v))).collect()),
        "numbering_restarts": json!(numbering.restarts.iter().map(|(a, b)| [a, b]).collect::<Vec<_>>()),
        "picture_bullets": J::Object(d.picture_bullets.iter().map(|(k, b)| (k.to_string(), json!({
            "relationship": b.relationship.as_deref(), "part": b.part.as_deref(), "pixels": b.pixels.map(|(w, h)| [w, h]),
        }))).collect()),
        "theme_font_lang": text_map(&s.theme_font_lang),
        "compatibility_mode": s.compatibility_mode,
        "default_tab_stop": s.default_tab_stop,
        "compat_options": texts(s.compat_options.iter()),
        "compat_settings": text_map(&s.compat_settings),
        "even_and_odd_headers": s.even_and_odd_headers,
        "hyphenation": s.hyphenation.map(|h| json!({"zone": h.zone, "limit": h.limit, "no_caps": h.no_caps})),
        "endnote_position": s.endnote_position.as_deref(),
        "endnote_named": ints(&s.endnote_named),
        "footnote_named": ints(&s.footnote_named),
        "footnotes": footnotes, "footnote_separators": footnote_separators, "footnote_separator_ids": footnote_separator_ids,
        "endnotes": endnotes, "endnote_separators": endnote_separators, "endnote_separator_ids": endnote_separator_ids,
        "stories": J::Object(d.stories.iter().map(|(k, v)| (k.to_string(), blocks(v, &d.names))).collect()),
        "story_parts": text_map(&d.story_parts),
        "cached_fields": texts(d.cached_fields.iter()),
        "warnings": texts(d.warnings.iter()),
    })
}

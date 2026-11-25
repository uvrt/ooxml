//! Owns: the table style new tables get, as Word's Table Design > Set as Default ("this
//! document only") sets it (docx-agent's `Styles.set_default`, round 31): the settings'
//! `w:defaultTableStyle` names it -- a built-in one the document lacks written first --
//! and nothing else changes; the same style again is no change.
//!
//! Must not: touch a table or another style.

use super::sheet::{self, make};
use super::StyleStep;
use crate::doc::Key;
use crate::names::W;
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;
use ooxml_word_model::parts::{related, REL_SETTINGS};

const SETTINGS_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml";

/// The settings part, made when the document has none (`_create_settings_part`).
///
/// # Errors
/// When a part cannot be added.
pub(crate) fn settings_part(sink: &mut EditSink<'_, '_>) -> Result<String> {
    let docx = sink.docx()?;
    if let Some(part) = related(sink.doc().package(), &docx.main, REL_SETTINGS)? {
        return Ok(part);
    }
    let main = docx.main.clone();
    sink.new_part(
        &main,
        ("word/settings.xml", "word/settings{n}.xml"),
        crate::new_part::word_root("settings"),
        SETTINGS_TYPE,
        REL_SETTINGS,
    )
}

/// `set_default` (docx-agent's `Styles.set_default`) of table style `name`.
pub(crate) fn set_default(sink: &mut EditSink<'_, '_>, space: &mut Space, name: &str) -> Result<StyleStep> {
    let docx = sink.docx()?;
    if let Some(part) = related(sink.doc().package(), &docx.main, REL_SETTINGS)? {
        sink.read(Key::part(part.as_str()));
    }
    let current = super::read::table_default(&docx);
    super::add::resolve(sink, name, "table")?;
    let found = sheet::find_in(&docx, name, Some("table"));
    if let (Some(current), Some(found)) = (&current, &found) {
        let id = ooxml_word_model::docx::style_id(current);
        if id == ooxml_word_model::docx::style_id(found) {
            let named = ooxml_word_model::docx::style_name(current).map_or_else(|| "None".to_owned(), repr_str);
            return Ok(StyleStep {
                id: Some(format!("style:{id}")),
                changed: false,
                warnings: vec![format!("{named} was already the default table style")],
                ..StyleStep::default()
            });
        }
    }
    let id = crate::builtin::ensure(sink, name, "table")?;
    let part = settings_part(sink)?;
    let held = sink.doc().clone();
    let shared = held.shared_handle();
    let orders = shared.facts.orders();
    let work = space
        .work(&held, &part)?
        .ok_or_else(|| EditError::no_part(part.clone()))?;
    let root = work.root();
    for node in work.children(root).to_vec() {
        if work.is(node, W, "defaultTableStyle") {
            work.remove(node);
        }
    }
    let made = make(work, "defaultTableStyle", &[("val", &id)]);
    crate::marks::insert_in_order(work, root, made, orders);
    Ok(StyleStep::of(Some(format!("style:{id}"))))
}

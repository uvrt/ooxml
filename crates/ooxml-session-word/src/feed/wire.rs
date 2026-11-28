//! Owns: the changes feed as the host hands it on ([`hosted`], `FormatDoc::changes`): asked
//! of the document's own page engine, and written in the design's wire shape (section 2,
//! with the rulings' `moved`) -- `{version, state, session, token, since, full, behind?,
//! parts, blocks: {changed, came, gone}, pages?: {list?, changed: [{page, order?, groups:
//! {id: {dy, svg}}, moved: {id: dy}, removed, defs: {id: markup}, pictures}]}}` -- each
//! picture by its content address, its bytes beside the changes, once.
//!
//! Must not: lay out or paint but through the feed, or put a picture's bytes in the changes.

use super::{asked, Ask, Changes, PageChange};
use crate::live_view::ViewError;
use crate::word::Word;
use ooxml_session::{FeedAnswer, FeedAsk, FeedBytes};
use ooxml_tools::{Refusal, ToolError, WayOut};
use serde_json::{json, Map, Value};

/// What changed in `word` after `ask`'s `since`, laid out by the document's own page engine.
///
/// # Errors
/// A refusal where the session lays the document out with no engine of its own; the feed's
/// (a version it does not lay out, past the limit on laying out).
pub(crate) fn hosted(word: &Word, ask: &FeedAsk<'_>) -> Result<FeedAnswer, ToolError> {
    let layout = word.own_pages().ok_or_else(|| {
        ToolError::refusal(
            Refusal::Refused,
            "the session lays out no pages: open the document in a session with page setup to read its changes \
             feed with pages, or ask without them (pages=false)",
            WayOut::one(Value::from(false)),
        )
        .at("pages")
    });
    let layout = match (layout, ask.pages) {
        (Ok(layout), _) => Some(layout),
        (Err(_), false) => None,
        (Err(refused), true) => return Err(refused),
    };
    let found = asked(
        &Ask {
            word,
            doc: ask.doc,
            layout: layout.as_deref(),
            nonce: ask.nonce,
            ports: ask.context,
            context: ask.call,
        },
        ask.since,
        ask.pages,
    )
    .map_err(|error| match error {
        ViewError::NotDrawn(tool) => tool,
        other => ToolError::internal(other.to_string(), "changes"),
    })?;
    Ok(answer(&found))
}

/// The changes in their wire shape, the pictures beside them.
fn answer(found: &Changes) -> FeedAnswer {
    let mut changes = Map::new();
    changes.insert("version".into(), Value::from(found.version));
    changes.insert("state".into(), Value::from(found.state));
    changes.insert("session".into(), Value::from(found.session.as_str()));
    changes.insert("token".into(), Value::from(found.token.as_str()));
    changes.insert("since".into(), found.since.as_deref().map_or(Value::Null, Value::from));
    changes.insert("full".into(), Value::from(found.full));
    if let Some(head) = found.behind {
        changes.insert("behind".into(), Value::from(head));
    }
    changes.insert("parts".into(), json!(found.parts));
    changes.insert(
        "blocks".into(),
        json!({"changed": found.blocks.changed, "came": found.blocks.came, "gone": found.blocks.gone}),
    );
    let mut pictures = Vec::new();
    if let Some(pages) = &found.pages {
        let mut out = Map::new();
        if let Some(list) = &pages.list {
            let frames: Vec<Value> = list
                .iter()
                .map(
                    |f| json!({"width": f.width, "height": f.height, "width_px": f.width_px, "height_px": f.height_px}),
                )
                .collect();
            out.insert("list".into(), Value::Array(frames));
        }
        let changed: Vec<Value> = pages.changed.iter().map(|p| page(p, &mut pictures)).collect();
        out.insert("changed".into(), Value::Array(changed));
        changes.insert("pages".into(), Value::Object(out));
    }
    FeedAnswer { changes, pictures }
}

/// One page's change in its wire shape; its new pictures' bytes added to `pictures`, once.
fn page(change: &PageChange, pictures: &mut Vec<FeedBytes>) -> Value {
    let mut out = Map::new();
    out.insert("page".into(), Value::from(change.page));
    if let Some(order) = &change.order {
        out.insert("order".into(), json!(order));
    }
    let groups: Map<String, Value> = change
        .groups
        .iter()
        .map(|g| (g.id.clone(), json!({"dy": g.dy, "svg": g.svg})))
        .collect();
    out.insert("groups".into(), Value::Object(groups));
    let moved: Map<String, Value> = change
        .moved
        .iter()
        .map(|(id, dy)| (id.clone(), Value::from(*dy)))
        .collect();
    out.insert("moved".into(), Value::Object(moved));
    out.insert("removed".into(), json!(change.removed));
    let defs: Map<String, Value> = change
        .defs
        .iter()
        .map(|(id, d)| (id.clone(), Value::from(d.as_str())))
        .collect();
    out.insert("defs".into(), Value::Object(defs));
    let hashes: Vec<&str> = change.pictures.iter().map(|p| p.hash.as_str()).collect();
    out.insert("pictures".into(), json!(hashes));
    for p in &change.pictures {
        if !pictures.iter().any(|q| q.hash == p.hash) {
            pictures.push(FeedBytes {
                hash: p.hash.clone(),
                mime: p.mime.clone(),
                bytes: std::sync::Arc::clone(&p.bytes),
            });
        }
    }
    Value::Object(out)
}

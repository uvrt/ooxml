//! Owns: a Word document's side of a co-authoring refresh ([`refreshed`]): another editor's
//! file opened, taken in by the editor ([`ooxml_word_edit::Editor::receive`]: the steps it
//! touched handed back, its blocks with their ids), and the session's records told of each
//! version it made (the ids each retired, by whom), in the session's terms.
//!
//! Must not: word a reply (the host does), or decide what is handed back (the store does).

use crate::word::Word;
use ooxml_opc::Package;
use ooxml_session::{FormatRefreshed, RefreshAsk, SessionError, TakenBack};
use ooxml_store::{DocModel, Seq};
use ooxml_tools::Meanwhile;
use ooxml_word_edit::{EditError, Key, WordDoc};

/// `asked`'s file taken into `word`.
///
/// # Errors
/// A base not kept ([`SessionError::RefreshBase`]); a file that is not a package; a
/// take-back the history refuses (another editor's step in the way).
pub(crate) fn refreshed(word: &Word, asked: &RefreshAsk<'_>) -> Result<FormatRefreshed, SessionError> {
    let from = Package::open(std::sync::Arc::clone(&asked.file), &asked.limits)?;
    let arrived = word
        .stored
        .held()
        .editor
        .receive(asked.editor, Seq(asked.base), from)
        .map_err(|error| match error {
            EditError::Store(error) => ooxml_session::store_error::<WordDoc>(error, Meanwhile::History),
            EditError::Package(error) => SessionError::Package(error),
            other => SessionError::format(other),
        })?;
    let undos: Vec<_> = arrived
        .handed_back
        .iter()
        .map(|h| (h.previous.clone(), h.version.clone()))
        .collect();
    let made = arrived.previous.as_ref().map(|previous| (previous, &arrived.version));
    word.stored.arrived(&undos, made, asked.editor)?;
    let handed_back = arrived
        .handed_back
        .iter()
        .map(|h| TakenBack {
            number: h.step.number,
            caller: h.step.caller.as_deref().map(str::to_owned),
            blocks: blocks(&h.step.writes),
            undo: (h.version.seq().0, h.version.number()),
        })
        .collect();
    Ok(FormatRefreshed {
        changed: arrived.previous.is_some(),
        version: Word::any(arrived.version),
        parts: arrived.parts,
        blocks: arrived.blocks,
        handed_back,
    })
}

/// The block ids among `keys`.
fn blocks(keys: &ooxml_store::KeySet<Key>) -> Vec<String> {
    keys.keys().map_or_else(Vec::new, |keys| {
        keys.iter()
            .filter(|key| matches!(key, Key::Block { .. }))
            .filter_map(WordDoc::key_name)
            .collect()
    })
}

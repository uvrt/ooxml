//! Owns: a session's documents as the application opens, saves, undoes and closes them
//! (ooxml-edit's `Session.open`, `save`, `undo`/`redo` and `close`, ported exactly): a
//! document opened from its bytes under an id the application gives (a resumed run's) or
//! the next one counted, with the state saved beside its bytes, its format found from them
//! where the application names none, within the session's room for documents ([`Opening`],
//! [`OpenRefused`], today's words); what the session keeps of each beside its versions
//! ([`Held`]: where it came from, its size); its bytes and state of one version
//! ([`Saved`]); the application's undo and redo; the files tools saved for it; and what
//! the application's own host (the Python host of pptx and xlsx, and the application's
//! tools, until Phase 4) keeps in the session's stores through it: datasets
//! ([`KeptDataset`]), outputs, the images shown.
//!
//! Must not: run a tool, or hold the stores' lock while a package is parsed or written.

use crate::session::HostSession;
use ooxml_base::pyfmt::repr_str;
use ooxml_session::blobs::check_package;
use ooxml_session::elsewhere::{ElsewhereFormat, HeldElsewhere};
use ooxml_session::outputs::Output;
use ooxml_session::{CallContext, DocId, DocKind, Format, SessionError};
use serde_json::{Map, Value};
use std::sync::Arc;

const MB: u64 = 1024 * 1024;

/// The room a session has for open documents (ooxml-edit's `Limits`' documents).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentLimits {
    /// How many may be open at once.
    pub max_documents: usize,
    /// Their bytes together, as opened.
    pub max_total_document_bytes: u64,
}

impl Default for DocumentLimits {
    fn default() -> Self {
        DocumentLimits {
            max_documents: 4,
            max_total_document_bytes: 200 * MB,
        }
    }
}

/// What the session keeps of an open document beside its versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    /// Where it came from: the blob it was opened from (`b1`), `new`, or `app`.
    pub source: String,
    /// Its bytes as opened.
    pub size: u64,
}

/// How the application opens a document.
#[derive(Debug, Clone, Copy, Default)]
pub struct Opening<'a> {
    /// Its kind; `None`: found from its bytes and name.
    pub kind: Option<DocKind>,
    /// Where it came from (`app` by default; a blob's handle, `new`).
    pub source: Option<&'a str>,
    /// The id it opens under (`d3`, a resumed run's); `None`: the next one counted.
    pub doc_id: Option<&'a str>,
    /// The state saved beside its bytes ([`Saved::state`]); `None` or empty: the bytes alone.
    pub state: Option<&'a str>,
}

/// Why a document was not opened.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenRefused {
    /// The application's argument: an id that is no id or is in use, a kind no format
    /// opens, a state refused.
    #[error("{0}")]
    Argument(String),
    /// A limit broken: the package's, or the session's room for documents.
    #[error("{0}")]
    Limit(String),
    /// Bytes no format opens: none takes them, or the one that does cannot read them.
    #[error("{0}")]
    Unreadable(String),
    /// The state given beside the bytes refused: a format that keeps none, or a state that
    /// is not these bytes' (the format's words say what to pass instead).
    #[error("{0}")]
    State(String),
}

/// An open document's bytes and the state to open them again with, of one version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    /// Its id.
    pub doc: String,
    /// Its name.
    pub name: String,
    /// Its kind.
    pub kind: DocKind,
    /// The version saved.
    pub version: u64,
    /// The bytes.
    pub data: Vec<u8>,
    /// Its format's state beside them; `None` where the format keeps none.
    pub state: Option<String>,
}

impl HostSession {
    /// Open a document from `bytes`, named `name`, as `how` says: its id (`d1`), its refs
    /// empty.
    ///
    /// # Errors
    /// [`OpenRefused`], in today's words.
    pub fn open_with(&self, bytes: Arc<[u8]>, name: &str, how: &Opening<'_>) -> Result<DocId, OpenRefused> {
        let given = how.doc_id.map(given_doc_id).transpose()?;
        if let Some(id) = &given {
            self.free_id(id)?;
        }
        let format = self.format_for(&bytes, name, how.kind)?;
        let state = how.state.filter(|s| !s.is_empty());
        let kind = format.kind();
        check_package(&bytes, &self.host.settings.blobs, name).map_err(|e| OpenRefused::Limit(e.0))?;
        let size = bytes.len() as u64;
        self.room_for_document(size)?;
        let id = self
            .session
            .open_as(bytes, kind, name, given.as_ref(), state)
            .map_err(|error| match error {
                SessionError::StateRefused { reason: None, .. } => OpenRefused::State(format!(
                    "{name} is a {} document, and that format keeps no state beside its bytes: open it without \
                     state",
                    kind.as_str()
                )),
                SessionError::StateRefused { reason: Some(why), .. } => OpenRefused::State(why),
                SessionError::IdTaken(id) => self.taken(&id),
                other => OpenRefused::Unreadable(other.to_string()),
            })?;
        let mut stores = self.stores();
        stores.refs.insert(id.to_string(), ooxml_session::RefTable::new());
        stores.docs.insert(
            id.to_string(),
            Held {
                source: how.source.unwrap_or("app").to_owned(),
                size,
            },
        );
        Ok(id)
    }

    /// A document the application's own host opened (design phase 3, decision D3): its id
    /// from the session's one counter (`doc_id`: the application's, as [`Opening::doc_id`]),
    /// a stand-in ([`HeldElsewhere`]) in the session's table, its `size` counted in the
    /// session's room, at state `version`.
    ///
    /// # Errors
    /// [`OpenRefused`]: an id refused or in use, a kind no format here holds elsewhere,
    /// no room.
    pub fn hold(
        &self,
        kind: DocKind,
        name: &str,
        size: u64,
        version: u64,
        how: &Opening<'_>,
    ) -> Result<DocId, OpenRefused> {
        let given = how.doc_id.map(given_doc_id).transpose()?;
        if let Some(id) = &given {
            self.free_id(id)?;
        }
        if !self
            .host
            .formats
            .iter()
            .any(|f| f.kind() == kind && f.as_any().is::<ElsewhereFormat>())
        {
            return Err(OpenRefused::Argument(format!(
                "{} documents are not held by the application's own host here",
                kind.as_str()
            )));
        }
        self.room_for_document(size)?;
        let held =
            Arc::new(HeldElsewhere::new(kind, size, version).map_err(|e| OpenRefused::Unreadable(e.to_string()))?);
        let id = self
            .session
            .insert_as(
                kind,
                name,
                Arc::clone(&held) as Arc<dyn ooxml_session::FormatDoc>,
                given,
            )
            .map_err(|error| match error {
                SessionError::IdTaken(id) => self.taken(&id),
                other => OpenRefused::Unreadable(other.to_string()),
            })?;
        let mut stores = self.stores();
        stores.refs.insert(id.to_string(), ooxml_session::RefTable::new());
        stores.docs.insert(
            id.to_string(),
            Held {
                source: how.source.unwrap_or("app").to_owned(),
                size,
            },
        );
        stores.held.insert(id.to_string(), held);
        Ok(id)
    }

    /// The application's own host's document `doc` is at state `version` now (a call or the
    /// application changed it, or undid a change): what its datasets' staleness and the
    /// rows read of it are compared with.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; [`SessionError::Kind`] for a document the session holds
    /// itself.
    pub fn held_moved(&self, doc: &DocId, version: u64) -> Result<(), SessionError> {
        self.session.kind_of(doc)?;
        let held = self.stores().held.get(&doc.to_string()).cloned();
        let held = held.ok_or_else(|| SessionError::Kind(format!("{doc} is held by the session itself")))?;
        held.moved(version);
        Ok(())
    }

    /// What `open_document` and `new_document` say of the open document `id`: its id, kind,
    /// name, where it came from and its version.
    pub(crate) fn described(&self, id: &DocId) -> Map<String, Value> {
        let mut out = Map::new();
        let opened = self.session.snapshot(id).ok();
        let source = self.stores().docs.get(&id.to_string()).map(|h| h.source.clone());
        out.insert("doc".into(), Value::from(id.to_string()));
        out.insert("kind".into(), Value::from(opened.as_ref().map(|o| o.kind().as_str())));
        out.insert("name".into(), Value::from(opened.as_ref().map(|o| o.name().to_owned())));
        out.insert("source".into(), Value::from(source));
        out.insert("version".into(), Value::from(opened.map(|o| o.number())));
        out
    }

    /// The open document `doc`'s bytes at its head and the state to open them again with,
    /// both of that one version. Writes nothing anywhere.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the package's error writing a part.
    pub fn save(&self, doc: &DocId) -> Result<Saved, SessionError> {
        let head = self.session.snapshot(doc)?;
        let format = self.session.format_doc(doc)?;
        Ok(Saved {
            doc: doc.to_string(),
            name: head.name().to_owned(),
            kind: head.kind(),
            version: head.number(),
            data: format.save(head.any(), false)?,
            state: format.state(head.any()),
        })
    }

    /// The application's undo (`redo`: redo) of up to `steps` calls on `doc` (with a caller,
    /// its own): how many it took back; the refs those it took back defined go with them.
    /// Nothing to take is 0.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; [`SessionError::Entangled`] when the caller's latest call
    /// is not the document's latest change.
    pub fn history(
        &self,
        doc: &DocId,
        steps: usize,
        redo: bool,
        caller: Option<&Map<String, Value>>,
    ) -> Result<usize, SessionError> {
        let context: CallContext = self.context(caller.filter(|c| !c.is_empty()), None);
        let done = if redo {
            self.session.redo(doc, steps, &context)?
        } else {
            self.session.undo(doc, steps, &context)?
        };
        if done.count > 0 {
            if let Some(table) = self.stores().refs.get_mut(&doc.to_string()) {
                table.restore(done.version);
            }
        }
        Ok(done.count)
    }

    /// Datasets the application keeps itself (ooxml-edit's `Session.keep_datasets`): each
    /// defined as a call would define it, a redefined id replaced; the serial each was kept
    /// under, in order.
    ///
    /// # Errors
    /// `limit`: the session would keep over its limit (nothing is kept).
    pub fn keep_datasets(
        &self,
        entries: Vec<ooxml_session::datasets::DatasetEntry>,
    ) -> Result<Vec<u64>, ooxml_tools::ToolError> {
        let heads = self.heads_now();
        let heads = |doc: &str| heads.iter().find(|(d, _)| d == doc).map(|(_, n)| *n);
        let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
        let mut stores = self.stores();
        stores.datasets.keep(entries, &heads)?;
        Ok(ids.iter().filter_map(|id| stores.datasets.serial(id)).collect())
    }

    /// The dataset `id` (no `d:`) as the application reads it ([`KeptDataset`]); `None`
    /// where there is none.
    #[must_use]
    pub fn kept_dataset(&self, id: &str) -> Option<KeptDataset> {
        let heads = self.heads_now();
        let heads = |doc: &str| heads.iter().find(|(d, _)| d == doc).map(|(_, n)| *n);
        let stores = self.stores();
        let entry = stores.datasets.find(id)?;
        Some(KeptDataset {
            serial: stores.datasets.serial(id)?,
            stale: entry.stale(&heads),
            closed: entry.closed(),
            source_version: entry.source.source_version(&heads),
            entry: entry.clone(),
        })
    }

    /// Forget the dataset `address` names (the application's housekeeping).
    ///
    /// # Errors
    /// `not_found`, naming the datasets there are.
    pub fn drop_dataset(&self, address: &str) -> Result<(), ooxml_tools::ToolError> {
        let retry = ooxml_tools::TryCall {
            tool: "drop_dataset".into(),
            arguments: Map::new(),
            fill: Vec::new(),
        };
        self.stores().datasets.drop_dataset(address, &retry)
    }

    /// Keep `output` for the application, as a tool of its own host saved it.
    pub fn add_output(&self, output: Output) {
        self.stores().outputs.add(output);
    }

    /// The files tools saved for the application since it last took them.
    pub fn take_outputs(&self) -> Vec<Output> {
        self.stores().outputs.take()
    }

    /// The files tools saved for the application since it last took them, kept for it.
    #[must_use]
    pub fn outputs(&self) -> Vec<Output> {
        self.stores().outputs.outputs().to_vec()
    }

    /// The images the session's budgets still allow now; `None` where neither caps.
    #[must_use]
    pub fn images_remaining(&self) -> Option<usize> {
        self.stores().images.remaining()
    }

    /// Refuse with `limit` when `count` images would pass a budget, counting none; with
    /// `reserve`, count them.
    ///
    /// # Errors
    /// The budget `count` passes.
    pub fn images(&self, count: usize, reserve: bool) -> Result<(), ooxml_tools::ToolError> {
        let mut stores = self.stores();
        if reserve {
            stores.images.reserve(count)
        } else {
            stores.images.check(count)
        }
    }

    /// Give back `count` images reserved that the model is never sent.
    pub fn release_images(&self, count: usize) {
        self.stores().images.release(count);
    }

    /// Each open document's id and its head's number now.
    fn heads_now(&self) -> Vec<(String, u64)> {
        self.session
            .documents()
            .into_iter()
            .filter_map(|id| Some((id.to_string(), self.session.snapshot(&id).ok()?.number())))
            .collect()
    }

    /// The format of `bytes` named `name`: `kind`'s where it is given, else the first that
    /// takes them.
    fn format_for(&self, bytes: &[u8], name: &str, kind: Option<DocKind>) -> Result<&Arc<dyn Format>, OpenRefused> {
        let formats = &self.host.formats;
        let mut kinds: Vec<&str> = formats.iter().map(|f| f.kind().as_str()).collect();
        kinds.sort_unstable();
        if let Some(kind) = kind {
            return formats.iter().find(|f| f.kind() == kind).ok_or_else(|| {
                let listed: Vec<String> = kinds.iter().map(|k| repr_str(k)).collect();
                OpenRefused::Argument(format!(
                    "no format {}; one of [{}]",
                    repr_str(kind.as_str()),
                    listed.join(", ")
                ))
            });
        }
        formats.iter().find(|f| f.detect(bytes, name)).ok_or_else(|| {
            let listed = if kinds.is_empty() {
                "none registered".to_owned()
            } else {
                kinds.join(", ")
            };
            OpenRefused::Unreadable(format!("{name} is not a document of any registered format ({listed})"))
        })
    }

    /// Refuse `id` where a document has it, saying which and how to free it.
    fn free_id(&self, id: &DocId) -> Result<(), OpenRefused> {
        if self.session.kind_of(id).is_ok() {
            return Err(self.taken(&id.to_string()));
        }
        Ok(())
    }

    /// The refusal of the id `doc`, which a document has.
    fn taken(&self, doc: &str) -> OpenRefused {
        let Some(held) = DocId::parse(doc).and_then(|id| self.session.snapshot(&id).ok()) else {
            return OpenRefused::Argument(format!("doc_id {} is in use", repr_str(doc)));
        };
        let others: Vec<String> = self
            .session
            .documents()
            .into_iter()
            .map(|d| d.to_string())
            .filter(|d| d != doc)
            .collect();
        let mut message = format!(
            "doc_id {quoted} is in use: {} ({}) is open as {doc}; close it first (session.close({quoted}), after \
             saving it if it changed), or open this one without doc_id for the next free id",
            held.name(),
            held.kind().as_str(),
            quoted = repr_str(doc)
        );
        if !others.is_empty() {
            let mut open = vec![doc.to_owned()];
            open.extend(others);
            message.push_str(&format!(" (open now: {})", open.join(", ")));
        }
        OpenRefused::Argument(message)
    }

    /// Refuse a document of `size` bytes past the session's room.
    fn room_for_document(&self, size: u64) -> Result<(), OpenRefused> {
        let limits = self.host.settings.documents;
        let stores = self.stores();
        let open = stores.docs.len();
        if open >= limits.max_documents {
            return Err(OpenRefused::Limit(format!(
                "{open} documents are open; at most {}: close_document one you are done with (save it first if it \
                 changed)",
                limits.max_documents
            )));
        }
        let used: u64 = stores.docs.values().map(|h| h.size).sum();
        if used + size > limits.max_total_document_bytes {
            return Err(OpenRefused::Limit(format!(
                "open documents would total {} bytes; at most {}",
                used + size,
                limits.max_total_document_bytes
            )));
        }
        Ok(())
    }
}

/// `doc_id` as an application gives it, checked: `d1`, `d2`...
fn given_doc_id(doc_id: &str) -> Result<DocId, OpenRefused> {
    DocId::parse(doc_id).ok_or_else(|| {
        OpenRefused::Argument(format!(
            "doc_id {} is not a document id: 'd' and a whole number from 1 with no leading zero ('d1', 'd2', \
             'd17'), the form the session gives and addresses such as 'd2/p:A' read",
            repr_str(doc_id)
        ))
    })
}

/// A dataset kept in a session as the application's own host reads it: the entry, the
/// serial it was kept under (never given to another keeping: a copy read of it is that
/// keeping's alone), and whether it is stale and its source closed, now.
#[derive(Debug, Clone)]
pub struct KeptDataset {
    /// The entry.
    pub entry: ooxml_session::datasets::DatasetEntry,
    /// Its serial ([`ooxml_session::datasets::DatasetStore::serial`]).
    pub serial: u64,
    /// Whether it is stale now.
    pub stale: bool,
    /// Whether its source document is closed.
    pub closed: bool,
    /// Its source document's version now; `None` once closed, or with no source.
    pub source_version: Option<u64>,
}

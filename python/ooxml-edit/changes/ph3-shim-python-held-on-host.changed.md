- **A toolbox whose formats include one the Rust host holds runs on it, beside formats the Python host
  holds**, provided each of those is of a kind the Rust session names (`pptx`, `xlsx`). Python-held
  documents have these rules (Phase 3, decision D3):
  - Their ids come from the Rust session's one counter, and its room counts them.
  - Datasets derived from them go stale as they change.
  - A call is routed by the documents it names.
  - A Rust-hosted call reads such a document's rows as resolved in Python first (the values
    channel).
  - `Session.documents` maps each id to the entry of a document the Python host holds, or to a
    `HostDocument` (id, kind, name) of one the Rust host holds.

- **`Result.changed_parts`** (co-authoring request 6). Beside `touched`, every result lists
  the zip parts the call changed, `{"doc": "d1", "part": "ppt/slides/slide1.xml"}` each, by
  document and name, for the application (never in `to_json`): the change's own parts
  (`history.Change.parts`; for an owned document the owner's `parts_between` the step's
  heads -- docx-agent's Rust store, by identity, not a diff). `[]` for a read, a save, an
  error; a `batch` has the union.

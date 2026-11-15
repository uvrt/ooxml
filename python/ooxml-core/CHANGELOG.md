# Changelog

ooxml-core is the Rust core of the ooxml packages (`crates/`), as the Python module
`ooxml_core`. Its version is the `crates/python` crate's.

## 0.4.0 -- unreleased

- **`ooxml_core.field_pages(doc, version, keys, info=False)`**: what a fields update reads
  of one laid-out state of a document opened with `open_document`, at exactly that version
  (a commit's sequence number; the interpreter lock let go throughout): the first page
  (1-based, blank pages counted) each paragraph or table key is drawn on, `null` for one
  not drawn, the page count, and with `info` every page's `[number, section, story kind
  (0 default, 1 first, 2 even), blank]` (`ooxml_word::fieldpages`). `None` where that
  version's snapshot is no longer kept. docx-agent 0.1.117's `word_fields` update reads
  its pages here.

## 0.3.0 -- unreleased

- **A snapshot no longer the latest lets go of what a check made of it.** The owner tells
  the format when it publishes a newer snapshot (`Format::retire`); a Word snapshot then
  drops its blocks' and paragraphs' key maps (made again if a check of that version comes),
  so the 64 kept snapshots no longer each hold their own (several MB each at ~3,000 pages).

## 0.2.0 -- unreleased

- **`ooxml_core.check(doc, version, request, seen=None)`**: docx-agent's check of some
  ranges, answered on the snapshot of exactly that version of a document opened with
  `open_document` (the interpreter lock let go throughout): the ranges' pages, extent,
  breaks and keeps, `since_last_check` against the snapshot `seen`, the layout facts
  (`ooxml_word::facts`: docx-agent's `layout_facts` rules, ported one for one, reading the
  range's pages and the records docx-agent made of each block), the fields facts, and the
  pages of some blocks. Returns the answer (JSON) and the snapshot, or `(None, None)` where
  that version's snapshot is no longer kept. What it does not port answers
  `{"fallback": why}` and docx-agent checks its own way.
- **Each version is laid out and published on its own** (a Word document's owner): a
  check reads exactly the version it asks for; the latest 64 snapshots are kept.
- The export and change sets carry each block's record (`rec`), what a paragraph, a run and
  a table draw (`draw`, for `since_last_check`) and the document's constants; a change set
  may carry new constants and sections.
- `open_document` reports a document it cannot read or lay out as an error rather than
  stopping the process.
- JSON keeps the order docx-agent writes its results in (serde_json's `preserve_order`).

## 0.1.0

- The first version: Word's layout (measure, paginate, the page-number fields) behind a
  document's owner, as moved in from the performance lanes' prototype.

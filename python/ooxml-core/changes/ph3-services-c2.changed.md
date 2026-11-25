- Datasets, sources and the session's stores as Rust session services (phase 3, C2): a dataset as a typed
  value in the shared tool contract (columns, rows of values, each value's sources, keys, type labels, the
  formula caches it was read from, number formats, Markdown worked out when first asked for, `from_records`, a
  run of its rows `d:x/#9:#16`); the session's dataset store in the order defined, within its limits of 50
  datasets and 50,000 cells, with staleness against each source's version now (undo makes a dataset current
  again, a scoped undo leaves it stale, a close keeps it as it was, a join's other sources count too) and the
  stale warning on every use; a dataset kept only when its call succeeds. A source qualified with its document
  (`t:x` as `d2/t:x`, with today's three refusals), read through the document's format (`FormatDoc::source`;
  docx takes none), and refused with only the sources that resolve. Blobs (checked by their bytes, images by
  their header, packages by their central directory), outputs and the image budgets. Nothing dispatches
  through them yet: tool output is unchanged.

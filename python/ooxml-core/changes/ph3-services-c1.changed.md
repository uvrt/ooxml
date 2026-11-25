- Refs and text facts as Rust session services (phase 3, C1): a document's refs by state number with their
  owners (undo, redo and scoped undo bring back each state's refs; refs outlive a close), their resolution on a
  tool's declared paths (`$name/rest`, a span's ends, the camelCase fold and its note, the misspelt suggestion,
  "was to be made by ops[N], which failed", the refusal to point another caller's ref elsewhere), the ref-name
  rule in words, and `{{d:...}}`/`{{t:...}}` references parsed and written in the locale (at most 50 per
  text, braces that look like a reference refused). A source's values (`Cell`, `SourceRows`, a cell's label, a
  date pattern) move from the Word tools to the shared tool contract. Nothing dispatches through them yet: tool
  output is unchanged.

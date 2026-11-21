- **`AliasWhen(name, argument, value)`**, a `Tool.aliases` entry read as `name` only while
  `argument` has `value` (`edit_chart`'s `slide`, its `target` when adding). The definition
  check holds `name` and `argument` to parameters, and the Rust session's tool export writes
  the same entry as `{"name", "when": {argument: value}}` (`exported`, `of_export`), which
  docx-agent reads back as one.

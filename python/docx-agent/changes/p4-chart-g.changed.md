- **`edit_chart`'s `set_title` and `set_axis_title` are answered by the Rust session**
  (lane P4c18, part b2): a title made in order or taken away, its text written into its
  rich text with the formatting kept character by character as ooxml-edit's
  `replace_body_text` keeps it (the same `difflib` opcodes, now in `ooxml_base::difflib`),
  a title drawn from a cell written in its cache and its workbook cell, and the chart's
  `autoTitleDeleted` kept true. New title text in a document whose defaults name no
  language stays docx-agent's own.

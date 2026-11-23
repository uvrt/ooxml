# What docx2svg draws

docx2svg draws what the Rust core's layout lays out: the same pages docx-agent's `render`
draws. What that is, and what it refuses as not supported yet, is listed in docx-agent's
[SUPPORTED.md](../../docx-agent/src/docx_agent/tools/SUPPORTED.md).

A page drawn, but not yet as Word draws it, says so in a `drawn-not-as-word` warning; a
place laid out by a rule not measured against Word is an approximation in
`ConvertOptions.coverage`; a document the layout does not lay out yet raises `LayoutError`.

The Word measurements docx2svg's Python layout was held to are kept under
`crates/ooxml-oracle/word/observations` (registered in its `index.py`).

- A Python tool's step bridged to the Rust session hands over the view's id ledger too
  (`ooxml_core.HostIds`): the session's next edit draws the paraId docx-agent's own next draw
  gives (an id the tool drew and did not write is not drawn again), and a block the tool stamped
  is found by its old id (`t@body/0/c2,2` after a row insert) (lane P2e6).
- `docx_format(typography=False)`: a writing call is served by the session, its context saying
  the text is written as given (`ooxml_core.CallContext(typography=False)`); no
  `edit_fallbacks{reason="typography off"}` any more (lane P2e7).
- A served write whose text got its language's spacing says how many spaces it wrote as no-break
  spaces, as docx-agent's handler does (`word_controls` `fill` among them) (lane P2e7).
- A served `word_insert_markdown` whose `table_cell` style's indents run its text past the cell's
  edge, or leave it no width, says so with the style and where the indent comes from, as
  docx-agent's handler does (round 21, T4) (lane P2e7).
- **Served refusals as docx-agent words them (lane P2e8).** A served `word_edit_table` naming a
  workbook's table (`t:stock`, `d2/t:stock`) says it is a table of that document (round 56,
  R120); a served `word_insert_text` whose `ref` holds an address gives the call that places
  it (`details.call`) or the item (`details.item`).

- One table grid in the Rust core (`ooxml_word_model::docx::tables::Grid`, lane ph2-types2): the model, Markdown
  reading, addresses, tracked-change joins, `word_inspect`, the table reads, the originals watch and the layout's
  column facts place cells by one value rule and one cell walk instead of seven. Word 16.106 measured the rule
  (`grid-values-word.json`): `w:gridSpan`/`w:gridBefore`/`w:gridAfter` are decimal numbers with no white space
  (Word does not open a span of ` 2`, `1_0` or `-1`), a span of `0` is one column, a negative `w:gridAfter` none,
  and cells inside row-level content controls and custom XML take their grid columns. Documents Word opens read as
  before, except where a reader skipped wrapped cells or took a span of `0` as no column; see the PR.

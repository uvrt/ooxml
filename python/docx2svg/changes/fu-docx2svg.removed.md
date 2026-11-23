- `docx2svg.rust` no longer lays documents out: `Engine`, `RustLayout`, `RustPage`, `svgs`, `page_png`,
  `fallbacks` and `count_fallback` are gone, and with them the dual path where a version the Rust layout did
  not lay out was laid out by docx2svg's Python layout instead. The module keeps what a Rust session lays its
  documents out with (`page_setup`, `engine_key`, `memory_budget`, `word_fonts`); `page_setup` now raises
  `ValueError` for options the Rust layout does not take instead of answering `None`. docx2svg's own layout
  (`lay_out_docx`, `convert_docx_to_svg`) is unchanged.

  Instead: a Rust session's own page engine (`ooxml_core.Session.pages`, set with `page_setup`) lays a
  document out and renders it; a version it does not lay out yet is refused by the tools as not supported,
  with a way out.

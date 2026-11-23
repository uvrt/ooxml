docx2svg's Python layout is gone (lane svg-thin, cut-python-from-start): docx2svg is a thin layer over the
Rust core's layout and drawing (ooxml-core), the engine docx-agent's `render` draws with. Withdrawn, each
answering with what to use instead when asked for (`docx2svg.WITHDRAWN`, for one release): `lay_out_docx`,
`convert_docx_to_layout`, `page_svgs`, `parsed_document`, `drawn_faces`, `warm`, `scan_fonts`, and the
Python model and parser's names `Document`, `Paragraph`, `Run`, `Section`, `PageMargins`, `PageSize`,
`PageInfo`, `Package`, `parse_document`, `parse_package`. `ConvertOptions.advances`, `metrics`,
`decorations` and `names` are refused (`ValueError`) for one release, then removed. `svg_to_png` and
`convert_docx_to_png` take `backend="auto"` or `"resvg"` only (cairosvg is refused). docx2svg's submodules
(`layout`, `paginate`, `fonts`, `model`, `parse`, ...) were never public and go in the next release.
Instead: `convert_docx_to_svg`, `convert_docx_to_png` and `convert_docx` as before; for a layout to keep,
`docx2svg.rust.word_pages(options).lay_out(data)` (an `ooxml_core.LaidPages`: `.svg(page)`,
`.pngs(pages, raster)`, `.pages()`, `.page(n)`, `.coverage_summary()`; `WordPages.update(parts)` lays a
changed version out from the last one); for the model, docx-agent's `word_read` and `word_inspect`; for
measuring, `font_dirs` and `font_substitutes`; for cairosvg, rasterise `convert_docx_to_svg`'s pages
yourself.

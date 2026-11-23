# API

docx2svg is a thin layer over the Rust core (`ooxml_core`): every function lays the document
out with an `ooxml_core.WordPages` engine made for its options (`docx2svg.rust.word_pages`)
and draws its pages as docx-agent's `render` tool draws them.

- **`convert_docx_to_svg(source, options=None) -> list[str]`**: one SVG document per page
  `options.pages` selects (1-based; all by default). `source` is a path, bytes or a binary
  file.
- **`convert_docx(source, options=None) -> Conversion`**: the layout and the SVGs, laid out
  once. `Conversion.layout` is the `ooxml_core.LaidPages` (`.page_count`, `.pages()`,
  `.page(n)`, `.svg(n, width, height)`, `.pngs(pages, raster)`, `.coverage_summary()`),
  `Conversion.svgs` the selected pages' SVG, `Conversion.page_numbers` their numbers.
- **`convert_docx_to_png(source, options=None, *, backend="auto", font_dirs=None,
  font_files=None, skip_system_fonts=None) -> list[bytes]`**: one PNG per page, at 300 dpi
  unless `options.width`/`height` say otherwise, drawn with the files Word draws each face
  with (a face the document embeds included), then `font_files` and the folders
  `font_dirs` and `user_font_dirs(options)` name. `backend` is `"auto"` or `"resvg"`.
- **`svg_to_png(svg, *, width=None, height=None, scale=None, background=None, ...)`**: one
  SVG to PNG with the Rust core's raster.
- **`resolve_path(root, path)`**: the element a drawn element's `data-docx-path` names in a
  part's root element (ElementTree or lxml), or `None`.
- **`user_font_dirs(options)`**, **`glyph_sheet(glyphs)`**, **`embed_glyphs(svg, glyphs)`**.
- **`LayoutError`** (a `NotImplementedError`): the layout does not lay this document out
  yet; `reason` says what and where, `way_out` what to do instead.
- **`Warning(code, message, page)`**: collected in `ConvertOptions.warnings`.

`ConvertOptions` fields: `pages`, `width`, `height`, `glyph_size` (`device` or `exact`),
`font_dirs`, `substitute_fonts`, `font_substitutes`, `locale`, `text` (`text` or
`outlines`), `selectable_text`, `glyphs`, and what a conversion sets: `warnings` and
`coverage` (`docx2svg.coverage.Coverage`). `advances`, `metrics`, `decorations` and
`names` are refused (`ValueError`): the layout measures with the font files themselves.

- <a id="locale"></a>**Locale.** Word takes a few things from the machine it runs on
  rather than from the document: a decimal tab stop aligns on the *system's* decimal
  separator, not the document language's (measured: on a machine with a decimal comma,
  `1234.5` after a decimal stop in an `en-GB` document is drawn as after a right stop), and
  a chart's numbers are printed in its decimal and thousands separators. A renderer that
  read them from *its* machine would draw the same document differently on every server,
  so docx2svg never does: the caller says which locale a conversion is in
  (`ConvertOptions.locale`, an `ooxml_common.locales.Locale`), and without one it is
  `EN_US` -- a decimal stop aligns on `.`, a chart prints `1,240.0`.

  ```python
  from ooxml_common.locales import Locale

  nl = Locale(decimal_separator=",", thousands_separator=".", date_order="dmy",
              language="nl-NL", list_separator=";")
  svgs = convert_docx_to_svg("report.docx", ConvertOptions(locale=nl))
  ```

  `Locale` is frozen and picklable, so the options travel into worker processes
  unchanged. Only the separators are read so far: `DATE` and `TIME` fields and ordinal or
  cardinal number text are drawn as Word cached them, and a chart's own "Chart Title" is
  not drawn, each with a warning, whatever the locale.

## Withdrawn

docx2svg's Python layout was withdrawn (lane svg-thin). Asking for one of its names --
`lay_out_docx`, `convert_docx_to_layout`, `page_svgs`, `parsed_document`, `drawn_faces`,
`warm`, `scan_fonts`, `Document`, `Paragraph`, `Run`, `Section`, `PageMargins`, `PageSize`,
`PageInfo`, `Package`, `parse_document`, `parse_package` -- raises `AttributeError` saying
what to use instead (`docx2svg.WITHDRAWN`).

## Used by docx-agent

docx-agent imports these names from docx2svg, in its `src`, `tests` and `tools`, beyond the
functions above. `tests/test_public_api.py` lists the same names and fails when one is gone,
or when this list and that one disagree.

- `docx2svg` -- docx-agent: `ConvertOptions`, `convert_docx`, `convert_docx_to_svg`
- `docx2svg.rust` -- docx-agent: `engine_key`, `memory_budget`, `page_setup`, `word_fonts`

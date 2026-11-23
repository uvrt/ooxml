docx2svg's Python layout modules are gone (lane svg-thin, PR 2): `layout`, `paginate`, `relayout`, `linebreak`,
`lines`, `measure`, `fonts`, `hyphenate`, `model`, `parse`, `resolve`, `table`, `svg`, `drawing`, `chart`,
`diagram`, `fields`, `notes`, `floating`, `wrap`, `vertical`, `outlines`, `scope`, `recorded`, `opc`, `units`,
`xmlutil`. They were never public; the thin layer did not use them since the previous release. The
hyphenation patterns moved to the Rust core (ooxml-text). `docx2svg.coverage` keeps only `Coverage`, `StopInfo`
and `coverage_from`; `docx2svg.paths` only `resolve_path`; `docx2svg.errors.failure` is gone.
Instead: the public functions (`convert_docx_to_svg`, `convert_docx`, `convert_docx_to_png`, `svg_to_png`,
`resolve_path`), and `docx2svg.rust.word_pages(options).lay_out(data)` for the laid-out document
(`ooxml_core.LaidPages`).

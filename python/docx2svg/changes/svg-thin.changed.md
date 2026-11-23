docx2svg draws with the Rust core (lane svg-thin): `convert_docx_to_svg`, `convert_docx`,
`convert_docx_to_png` and the command line lay a document out with `ooxml_core.WordPages` and draw it as
docx-agent's `render` does: the same page SVG, and PNG drawn by the same raster with the same faces (at 300
dpi unless a size is given). Pages differ from docx2svg 0.14's where
the Rust layout follows Word more closely; Word is the reference. `Conversion.layout` is now the
`ooxml_core.LaidPages` (it was docx2svg's Python `Layout`). A document the layout does not lay out yet
raises `LayoutError`, now a subclass of `NotImplementedError` that says why and what to do instead. A page
drawn but not yet as Word draws it is a `drawn-not-as-word` warning. `resolve_path` resolves a path with
the Rust Word model's rules (`ooxml_core.element_path`). docx2svg now depends on ooxml-core and no longer
on resvg-py; the `png` extra installs nothing extra and is removed in the next release.

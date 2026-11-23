docx2svg's thin layer (lane svg-thin): `LaidPages.svg(page, width=None, height=None)` sizes a page;
`LaidPages.pngs(pages, raster, width=None, height=None, system=None)` draws pages as PNG with the faces
they draw, as the `render` tool draws them; `LaidPages.raster_files(pages)` gives the files a rasteriser is
handed for them; `LaidPages.coverage_summary()` gives the version's coverage. `WordFonts.face(family)`
says which installed file a face is laid out with, and `WordFonts.default_substitutes()` gives Word's
measured substitutes. `drawing_plan(svg, font_files, font_dirs)` gives the passes a text is drawn in where
two faces answer to one name, and `element_path(xml, path)` gives where a layout path's element is in a
part. A `PageRaster`'s own files are now handed over, after a document's faces, to the raster made for
its pages.

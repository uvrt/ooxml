Lane RD1: `ooxml_core.CallLayout(pages, *, raster=None, images_left=None, placer=None)` is
what a call reads of the document's layout. It goes to `CallOptions(layout=...)` in place of
the `placer=` and `layout=` keywords. `render` is a Rust tool (`tool_specs("docx")` lists it,
and its definition is docx-agent's byte for byte). It draws pages and sheets of pages from
the host's `WordPages`, with a document raster made from `raster` (the system's fonts and the
font folders) for the faces the pages draw. Images are kept per document within the session's
memory budget.

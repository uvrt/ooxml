Lane RD1: `ooxml_core.CallLayout(pages, *, raster=None, image_budgets=None, placer=None)` is
what a call reads of the document's layout. It goes to `CallOptions(layout=...)` in place of
the `placer=` and `layout=` keywords. `render` is a Rust tool (`tool_specs("docx")` lists it,
and its definition is docx-agent's byte for byte). It draws pages and sheets of pages from
the host's `WordPages`, with a document raster made from `raster` (the system's fonts and the
font folders) for the faces the pages draw. Images are kept per document within the session's
memory budget. `image_budgets` are the host's session and round budgets of
images as they stand (`[("session", limit, used), ("round", limit, used)]`); `render` refuses
images past them, worded as docx-agent words it. `tool_hosting("docx")` says `render` is
served with the host's page engine: a reading tool now declares its hosting too.

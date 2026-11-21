`docx2svg.rust` (lane LT1): a page laid out in Rust gives its text boxes' lines -- each
floating drawing's `lines`, in page coordinates -- and `RustPage.text_lines()` lists them in
paint order with the page's own, as docx2svg's Python layout does (ooxml-core's
`LaidPages.page`). Before, a paragraph in a text box was drawn in the page's SVG but was
not among its lines: docx-agent's `ids_on_page` left it out and `drawn_faces` missed its
faces.

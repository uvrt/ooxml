`check`'s keep trials measure the blank at a page's foot as docx-agent does: up to the
page's footnotes (their separator's line) and below the lines its body's text boxes draw,
which the layout's facts leave out. The Rust check said a page with footnotes was blank
down to its text area's bottom.

A `PAGE`, `NUMPAGES` or `SECTIONPAGES` field (and a `PAGEREF` to a bookmark) in a paragraph of
a body text box gets its result from the page the box draws that paragraph on, as docx-agent
and Word do; the Rust update left it empty and warned that the layout cannot say, the body's
lines not drawing the box's paragraphs.

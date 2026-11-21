`check`'s `reflow.coverage.approximations` from a Rust layout now lists each entry as
docx-agent does: a glyph a face lacks with its text as the `reason` and an empty `message`
(docx2svg's `coverage_of` records it so), any other by the part of its code after the colon
(the whole code where it has none). Before, a glyph's entry said `reason: "glyph"` and put its
text in `message`.

Lane H7d: `Version.block_kind(id)` says what kind of block an id names in a Word version:
`"paragraph"`, `"table"` or `"content control"` (a block-level one). A renamed id is followed.
It gives `None` when the id names none of the blocks. `Version.has_drawing(id)` says whether the
version has the drawing `d:N`.

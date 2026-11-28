- **A table's grid as accepted follows Word where pending cell deletions do not line up across rows.**
  - Word's Accept All merges a deleted cell that keeps something (text, or its paragraph mark)
    into the cell before it in its row, or the cell after it when it is the first; a deleted
    cell that keeps nothing goes, and the cells after it move left. Only then is the grid
    rebuilt. `word_read`'s Markdown, `word_inspect` and `word_edit_table`'s `columns` follow.
  - A cell named by row and column is that row's own cell there (a note says which grid
    column, where it moved). A whole column that is no one grid column in every row --
    `delete_column`, `insert_column`, a merge's rectangle, a column's format or fill, `sync`,
    `set_from` -- is refused as not supported, naming the rows that differ.
  - Reject All likewise removes a row whose cells are all pending insertions.

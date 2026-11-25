- A table whose `w:gridSpan` or `w:gridBefore` holds billions of columns (up to 2147483645, the largest Word
  opens, and larger) no longer takes tens of gigabytes in `set_cells`, a cell split or their "no cell at"
  refusal: a table's grid places are kept as one run of columns per cell, not one place per grid column.
  The refusal counts the places no cell covers instead of listing each, so it names the first ten and the
  rest's count as before. Normal tables read and refuse as before.

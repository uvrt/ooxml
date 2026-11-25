- The table tools (`word_edit_table`, `word_format_table`, `set_cells`, table sync and fills) read
  `w:gridSpan`, `w:gridBefore` and `w:gridAfter` by the one grid value rule, as Word reads them. Before,
  they parsed the value on their own. Replies change for odd values only, and now match Word:
  - `2.0` and `2.5` are two columns (they were one), so two values for both positions of such a cell are
    refused as one merged cell, with the split.
  - `" 2"` is one column (it was two).
  - `99999999999999999999` wraps to 1661992959 columns (it was one), and `4294967298` is two.
  - A `w:gridBefore` or `w:gridAfter` of `1.0` is one column (it was none), so such a row is no longer
    counted as off its grid.
- A table whose cell spans billions of columns no longer takes gigabytes in these tools or in a table's
  accepted view: the positions a cell taken into the one before it covers are kept as one run per cell.

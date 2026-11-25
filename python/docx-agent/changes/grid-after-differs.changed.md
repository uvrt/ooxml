- The table grid works out a row's trailing columns itself, the columns it leaves of `w:tblGrid`, and no
  longer reads `w:gridAfter` for coverage. Word does not read `w:gridAfter` either: it fills those columns
  and writes back the count (grid values probe, the `after-` cases). As a result:
  - a row that ends within its grid is on the grid whatever its `w:gridAfter` says, so `grid-row-mismatch`
    and check's `table_grid_mismatch` "off grid" entries are now only for rows whose cells reach past the
    grid. A short row is now listed as ragged where its cells do not line up.
    A table holding tracked cell or grid records (such as a `w:tblGridChange` widening the grid) is still
    read as written, so check keeps naming the change that leaves its rows short, and the reject that
    settles it (R128).
  - validate reports a written `w:gridAfter` that differs from the columns the row leaves, including a
    short row with none, as the new warning `grid-after-differs`.
  - Warnings are listed apart from problems: check's `validate` gives new ones under `warnings`, and they
    are not in `new`, `fixed`, `baseline` or `problems`. The save gate does not refuse a document for a
    new warning.

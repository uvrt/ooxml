- The editing core's table readers use the one table grid too (`GridSource`, lane ph2-types3): setting cells,
  a table's shape, deleting, adding and splitting columns, the row look, `validate`'s grid check, the
  `table_grid_mismatch` fact and Accept All / Reject All's grid tidying place cells by Word's value rule and cell
  walk. Documents Word opens read as before, except: a review's grid tidying takes a span of `0` as one column and
  `+2` as two, as Word does; and `validate` reports `grid-span-invalid` for every span Word does not open (`x`, an
  empty one, ` 2`), not only for those below 1. See the PR.

- Table grid values are read as Word 16.106 reads them (lane ph2-types4, measured by the grid probe): a
  `w:gridSpan` or `w:gridBefore` with a fraction counts its whole part (`2.0`, `2.9` and `2.` are 2, `.5` is 0), a
  number past 32 bits wraps as it does in Word (`99999999999999999999` is 1661992959, `4294967298` is 2), and an
  empty value, `+`, `-` or `.` is 0. `validate` no longer reports `grid-span-invalid` for spans Word opens (`2.0`,
  `99999999999999999999`); it does report spans Word refuses that it used to pass (`1e1`, `2147483646` and up, or a
  wrapped value below 0 such as `4294967295`), and `.` or `-0.5`, which Word reads as 0, like `0`. A row a wrapped
  span takes past the grid is reported as `grid-row-mismatch`. Rejecting a row-property revision now also drops a
  `w:gridBefore`/`w:gridAfter` of ``, `.` or `-0.5`, which Word reads as 0.

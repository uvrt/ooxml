- A table whose row reaches past both its `w:tblGrid` and the 63 columns a Word table takes, such as a
  `w:gridSpan` of 2147483645, is no longer widened to one grid column per column. That widening took tens of
  gigabytes:
  - Layout and render now report the table as not laid out: "row N reaches M grid columns, past the 63 a
    Word table takes".
  - Accepting or rejecting a change in such a table leaves its `w:tblGrid` as it is and warns about it.
  - Reading such a table's pending column deletions also no longer counts one entry per column.
  - Rows that reach past their grid by less are widened as before.

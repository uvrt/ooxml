- **A table's grid in the final view is the one Word writes.**
  - Grid columns that no cell edge divides, once the pending changes are accepted, are one
    column. `word_read`'s Markdown counts spans in those columns, render draws them as one,
    and `word_inspect` and `edit_table` count positions over them. A note says so: "grid
    columns 0 and 1 are one column in this view (no cell divides them)".
  - The grid spans to the widest row. A table written with rows past its `w:tblGrid` counts
    their columns (the outline's table `columns`), and a grid column past every row goes.
  - A vertical merge that only the recorded side of a pending cell properties change has
    stays merged in the final view, as it does in Word.

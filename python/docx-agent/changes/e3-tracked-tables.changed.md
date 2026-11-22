Lane E3: `word_edit_table`'s rows, columns, merges and splits of a table with pending tracked
structure (a cell deleted, inserted or merged, a row deleted) are served by the Rust session,
no longer left to the Python path ("pending tracked structure", "split tracked"): a new row's
cells follow the pending column changes beside them, a tracked row or column deletion gives
back the text a pending merge took from its cells, a tracked merge takes in a pending merge
whole (counted as made) and leaves out a row or cell whose deletion is pending, a tracked
split's new rows lose their cells under a pending column deletion; a tracked reshape that
would move a pending merge to another cell is refused naming the changes to settle first, by
id. A change that leaves the table ragged (as Accept All leaves it) says so, as docx-agent
said it.

Lane E1b: the Rust `word_edit_table` runs `sync` (a table kept in step with a source's rows by
key, the host handing the rows over as for `set_cells` `from`); `PlaceRows` adds rows beside any
row, deletes rows and keeps or drops the table's record in the same step.

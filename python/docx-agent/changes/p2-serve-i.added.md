Phase P2, lane P2c17: the last editing group runs in the Rust session (`rust.edits.SERVED`):
`word_changes`, `word_comments`, `word_controls`, `word_set_tracking` (the document's
tracking mode set in the session and on the entry together), `word_copy_from` (a source in
the same session) and `word_edit_table` for the actions that write no rows; set_cells,
insert_row, copy_row and sync stay with the Python handler (round 58's row look, R126, is
Python's).

Phase P2, lane P2c19: `word_edit_table` set_cells, insert_row and copy_row run in the Rust
session, round 58's R126 step with them (the rows written kept for check's
`row_text_format`, the column's formatting taken into a cell that was empty, the warning for
a row unlike the rows around it). The rows written are the session's record for a document
it owns (`rust.owner.SessionRows`): rows the Python handler writes (sync, `from`) go into it
too.

Lane U2: a call is one commit, so one undo step and one state number. `word_drawings
insert_picture` (its own paragraph, the alignment, the picture, the float) and the calls that keep
values written by reference (`word_set_text`, `word_insert_text`, `word_replace_text`,
`word_insert_markdown`) now run their commands in one transaction. Before this they committed
several times, and a part that refused late left the earlier parts in the document. Debug builds
check that a call commits at most once.
`word_edit_table`'s rows written (`set_cells`, `insert_row`, `copy_row`): the text written into
an empty cell given its column's formatting now runs in the transaction of the call's command, so
the call is one commit, one undo step and one state number; the rows are kept in the session's
record only once the call is committed, by the ids it drew.

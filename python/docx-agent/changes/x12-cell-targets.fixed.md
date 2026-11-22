- **Train 12's objects differences** (lane X12): `edit_chart`'s `add` on a row or cell id is
  refused as docx-agent refuses it (`invalid_arguments` naming the table and the cell's
  paragraphs), not charted in the cell's first paragraph; `word_edit_table`'s ragged warning
  reads the table's shape as docx-agent's `ragged_rows` does, a table off its grid too, so a
  change on a table already ragged no longer says it made it so.

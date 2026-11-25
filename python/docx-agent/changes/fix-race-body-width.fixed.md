- `word_format_table` with `width_of: "body"` works the body's lines out in its own edit, on the version it is
  applied to, and reads what they come from: the blocks from the paragraph beside the table to the table, the
  styles and numbering parts, and the section or the cell the table sits in. A paragraph put between the table and
  the paragraph before it by another caller at the same time is now a conflict, and the format is prepared again
  with the new paragraph's indents. Before, the lines were read from the head before the edit began, so the edit
  could commit a width worked out from the version before the insert. The room a width in percent or AutoFit to
  Window reads is now read by key too. No reply or refusal changes.

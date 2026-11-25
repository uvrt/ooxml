- The tools that plan an edit from what the document holds now make that plan in the edit's own transaction, on the
  version the edit is applied to, and read what it depends on there: the place a `find` names in `word_controls`,
  `word_drawings`, `word_links`, `word_notes`, `word_fields` and `word_comments`; `word_format`'s `find` ranges
  and its `like` sample's look; `replace_text`'s "exactly one match" check; `word_lists`' check of paragraphs
  their style numbers; a heading's section in `word_move` and `word_copy_from`, and `word_move`'s
  `after_section`; and `after: "end"` in `word_move`, `word_sections` and `word_fields`. An edit by another caller
  at the same time that moves the found text, makes a second match, changes the sample, puts a block into the
  section or at the end is now a conflict, and the call is planned again on the newer version, with the result
  of the two calls made one after the other. Before, the plan was made on the head before the edit began, so the
  edit could apply offsets, ids or a last block from the version before the other edit. `word_delete`'s report
  on the file's original text now reads the version the delete made, not a later one. No reply or refusal
  changes.

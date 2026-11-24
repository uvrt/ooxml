- `word_sections insert_break` with tracking on gives the split section's header and footer references to
  the new section, as it already did untracked and as Word 16.106 writes a tracked break: the section after
  the break names none and inherits them. Before, both sections named the same header and footer; where
  that header held a chart, Word showed an alert and did not open the file (Word suite, `chart-places.docx`).
- `word_changes` reject of a tracked section break (and accept of a tracked removal) hands the gone section's
  header and footer references on to the section after it where that names none of its own, as Word 16.106's
  Reject All does: the section left shows its header again. Before, the references went with the break.
  The tracked break's result now also lists `{"part": "word/document.xml"}` without a section next to the
  break's heading section, as the untracked one always has: the last section's properties changed (a change
  in no block, round 16 F12).

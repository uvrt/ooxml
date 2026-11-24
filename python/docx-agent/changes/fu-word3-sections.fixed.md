- `word_sections insert_break` with tracking on gives the split section's header and footer references to
  the new section, as it already did untracked and as Word 16.106 writes a tracked break: the section after
  the break names none and inherits them. Before, both sections named the same header and footer; where
  that header held a chart, Word showed an alert and did not open the file (Word suite, `chart-places.docx`).

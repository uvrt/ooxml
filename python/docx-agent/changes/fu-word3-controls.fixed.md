- `word_changes` accept or reject that empties an inline content control leaves it showing its placeholder,
  as Word 16.106's Accept All and Reject All do: `w:showingPlcHdr` set and, for a control naming no
  placeholder text, five spaces as its content. Before, the control was left empty, and the reject view
  differed from Word's on every edit of `controls.docx` (Word suite, control 309).

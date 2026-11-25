- `word_inspect` lists a paragraph whose mark is a pending deletion as `joins` on the next block, whatever
  that block is. Before, `joins` was only on a following paragraph. When the next block was a table or a
  content control, the paragraph was listed only inside it, on the first cell's or the control's first
  paragraph, and not among the body's blocks. A host that maps the body's blocks to sections could not
  place it. Now:
  - a table lists it in `joins` (Accept All joins it into the first cell);
  - a content control lists it in `joins` (Accept All joins it into the control's first paragraph);
  - an empty control passes it on to the block after the control.

  The paragraph that takes the text in keeps its own `joins`. The final view is unchanged: as in Word's Accept
  All (Word 16.106, measured), a deleted paragraph before a table or a control goes, and the next block keeps
  its text.

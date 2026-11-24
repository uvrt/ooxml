- `word_copy_from` and `word_move` refuse to put blocks holding a footnote or endnote reference in a text
  box, as Word 16.106 has no note reference there (its Insert Footnote is off in a text box, and its paste
  there loses the reference): the refusal's options are the places before and after the paragraph
  anchoring the text box, as `word_notes` refuses a reference there. Before, the reference went into the
  text box (`word_move`: into its drawing and its VML fallback both, as `word_notes move` did before #354).
  Blocks without a note reference still go into a text box.

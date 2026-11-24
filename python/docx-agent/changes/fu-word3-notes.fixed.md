- `word_notes insert` and `move` refuse a place in a text box, as Word 16.106 puts no note reference there;
  the refusal's options are the places beside the text box in the paragraph anchoring it (before the text
  box, the paragraph's end). Before, a reference moved there went into the text box's drawing and its VML
  fallback both, and Word did not open the file (Word suite, `chart-places.docx`).

- `word_fields insert_cross_reference` refuses a text cross-reference inside the bookmark it shows, as Word
  16.106's update of a `REF` inside its own bookmark gives an error result ("Invalid bookmark reference"),
  whether the bookmark stands around a content control or in a plain paragraph. The refusal's options are
  the end of the last paragraph before the bookmark and the start of the first after it; a page
  cross-reference (`kind: page`) there is not refused, as Word resolves it. Before, the tools wrote the
  bookmark's first line as the result (Word suite, `controls.docx`).

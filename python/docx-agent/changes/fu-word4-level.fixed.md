- A list paragraph at a level its list's definition lacks reads as Word shows it: Word 16.106 fills a
  multi-level definition's missing levels when it opens the file, each a decimal level with no text (no
  label), so `word_read` reads such a paragraph as Word's numbered level rather than a bullet from its
  style. `word_lists level` writes what Word's own tracked level change writes (the paragraph's `w:numPr`
  and a `w:pPrChange`, nothing in the numbering part), as it did (Word suite, `style-document.docx`).

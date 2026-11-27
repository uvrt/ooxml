- A Strict Open XML document (Word's Save As "Strict Open XML Document") is refused by every tool but
  `render` (`not_supported`; the way on: save it as a Word Document (.docx) in Word first). Before, the
  reading tools answered as if the document were empty, and an edit wrote into it what Word refuses to
  open (the insert went after the body and added a second styles part; Word 16.106 asks to repair the
  file). `render` draws it as it draws the same document saved as a Word Document, strict's measures
  (`35.40pt`) read as Word reads them; before, its headers, footers and notes were left out and its
  page size, margins, indents and spacing were read as absent.

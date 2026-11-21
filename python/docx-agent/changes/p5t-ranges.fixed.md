Lane P5t: `check(range=[...])` answered by the Rust session refuses a range as docx-agent does
(`not_found` for one that names nothing or holds no paragraph, with the body's headings as the
ways in) instead of handing it back; names a heading's range as the span it is
(`h:p:A` -> `p:A..p:C`); counts a text box's lines in its paragraph's extent; and reports
`one_page`, `kept`, `breaks` and `fix` for a paragraph of a table's cell given alone, and for a
span of paragraphs holding text boxes.

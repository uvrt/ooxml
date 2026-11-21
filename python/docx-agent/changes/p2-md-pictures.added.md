Phase P2, lane P2e5: `word_insert_markdown` given a `blob` (its UTF-8 text as the Markdown)
or Markdown showing pictures (`![alt](b1 "title")`: a blob by handle or name, or a `data:`
URI) is served: the session reads the session's blobs (its `Hosting` declares them) and
writes each picture inline as docx-agent's `Writer.picture` does -- its media part, its
`docPr` number, its name the title or `Picture N`, scaled to the text's width where wider --
with docx-agent's refusals (a remote source, a blob that is no image or is missing, bytes
that are no picture, a blob that is not UTF-8), a refusal of the place first.

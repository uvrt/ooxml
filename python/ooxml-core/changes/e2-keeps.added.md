Lane E2: `keep_together` is the Rust session's in `word_format` and `word_format_table`. A span,
table or section `word_format` keeps on one page that the version before the change is drawn
taller than a page in -- where the document's last check laid that very version out -- is said
so ("cannot be kept on one page"); nothing is laid out for it. `word_format_table` takes the
document's page engine (its hosting declares the layout) and lays the version it made out for
the "taller than a page" warning; where no layout is handed, or laying out stops or is not
done, the keep is made and the warning says its pages are not known, as docx-agent says it.
`Session.block_pages(doc, blocks, options)`: the pages the head version draws each body block on,
numbered from 1 as the session's other page reads number them, laid out by the options' page
engine within the call's limit on laying out (`("pages", numbers, None)`, `("outside", ...)`,
`("unknown", None, why)`, or no blocks and why nothing was laid out). Inside a step group
`word_format_table` leaves a table kept together unmeasured, for the host to ask after its batch.

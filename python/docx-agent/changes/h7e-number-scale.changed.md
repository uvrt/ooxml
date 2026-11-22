Lane H7e: for a document the Rust session owns, the format's `number_texts` and `scale_values`
hooks no longer read the Python view. They read the call's subject (ooxml-edit's
`OwnedDocument`) through the session (`docx_agent.rust.hooks`): the texts a check of numbers
reads and the values a scale's targets hold, each with the step it is styled as now, as the
session reads the version, placed on their pages by the session's own engine for the document.
`tools._base.refs_resolved` resolves refs in an address without reading a heading's range.
The engine is set to the document's own font folders and locale. A check of ranges the
document's Rust owner (lane F) would answer goes to docx-agent's handler where the last check
was the session's, so its `since_last_check` is measured from what that check saw.

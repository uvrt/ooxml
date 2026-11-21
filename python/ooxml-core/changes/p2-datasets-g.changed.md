- **`ooxml_core.CallOptions` (lane P2d3).** `Session.call(name, arguments, options)` and
  `Session.edit(name, arguments, options)` take what one call runs with as one
  `CallOptions(context=, limit=, documents=, placer=, layout=, blobs=, written=)` in place of
  their growing keyword arguments (the `too_many_arguments` allowances gone); `edit`
  refuses options with a `limit`. What a call needs about the other open documents it names
  is one `CallDocuments(at=, names=)` (the versions it reads, the host's names for them; lane
  P2c17's `names=` keyword moved here). docx-agent's callers moved to it.

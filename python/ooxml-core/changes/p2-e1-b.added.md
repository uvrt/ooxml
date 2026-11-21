- **A refusal of a removed id in docx-agent's words (lane P2e1).** A call refused `not_found`
  that names an id a version removed says which change removed it, from the document's
  retired-id record (a block one block replaced: "replaced with <id>", "or use <id>");
  `Session.number_version(doc, version, number)` takes the host's number
  of a version.
- **A call's date from the host (lane P2e1).** `CallContext(writing=CallWriting(date=...))`: the moment the call's
  writes are dated by, UTC to the second; without one, the session's clock, now to the second
  too (was: to the minute).
- **A call's typography from the host (lane P2e7).** `CallWriting(typography=False)`: the text
  the call writes is written as given, none of the spacing its language wants (docx-agent's
  `docx_format(typography=False)`); by default it is spaced, and the reply says how many spaces
  it wrote as no-break spaces. A served `insert_markdown` names a `table_cell` style whose
  indents do not fit its cells (round 21, T4).
- **`ooxml_core.CallWriting` (lane P2e8).** How a call writes, its date and typography, grouped in
  one `CallContext` setting beside main's reply limits.

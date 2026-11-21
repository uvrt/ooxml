- **A refusal of a removed id in docx-agent's words (lane P2e1).** A call refused `not_found`
  that names an id a version removed says which change removed it, from the document's
  retired-id record; `Session.number_version(doc, version, number)` takes the host's number
  of a version.
- **A call's date from the host (lane P2e1).** `CallContext(date=...)`: the moment the call's
  writes are dated by, UTC to the second; without one, the session's clock, now to the second
  too (was: to the minute).

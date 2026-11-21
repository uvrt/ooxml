Phase P2, lane P2e1: an id a change removed is refused by the Rust session in docx-agent's
words (which change removed it, the version as the tool layer numbers it, who made it, and the
way on), the host only passing the refusal through; the tool layer gives the session each
version's number as it numbers it (`Session.number_version`). A served call's writes (a
tracked edit's revision, a comment) are dated by the session's clock to the second
(`CallContext(date=...)`), so a session with its own clock is served by the Rust session, no
longer answered by the Python handler.

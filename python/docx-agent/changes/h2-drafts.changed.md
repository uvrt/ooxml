Lane H2: no "a draft" or "a document with a draft" fallback for a served edit. ooxml-edit
prepares no call of a document the Rust session owns on a draft, so those conditions never
held; they are gone from `rust/edits`. Another document that a call holds as a reader of a
committed version of an owned document now names that document in the session, instead of
falling back as "a document the session does not own".

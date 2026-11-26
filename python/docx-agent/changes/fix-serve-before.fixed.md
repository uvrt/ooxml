- Concurrent writers on one document: a `word_*` edit no longer fails with an internal
  `TypeError: 'None' is not an instance of 'Version'` when the others make 50 or more steps
  between its edit and its reply (ooxml-edit's fix; the session hands back the version the step
  went from, held).

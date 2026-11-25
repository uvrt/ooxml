- An address is read once where a tool takes it (lane ph2-types6, architecture review R1 #3, step 7): an
  `Address` keeps the text as given, the form it is written in and every form its text could also be read as,
  and a `Place` reads `end`, `end:<story>`, `after:`, `before:`, `replace:` or a bare address. `insert_markdown`
  and `copy_from` take their place as a `Place`; inserting and deleting text, placed edits and hyperlinks take
  their range as an `Address`; the reads (`word_read`, `word_inspect`, `find_text`, review listings, `check`)
  resolve an `Address`, and a heading's range expands to the `Address` of its span. Refusals keep their wording
  and order: a test resolves every address of the gates' cases both ways and finds them the same. No reply
  changes.

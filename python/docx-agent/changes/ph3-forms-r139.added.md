- `replace_text` takes `range` and `regex` again (round 60, R139), alone or together:
  - `range` replaces only in the paragraphs the range holds: a block or a span (`p:A..t:B`, `p:A..`,
    `..t:B`), a table cell (`t:X/c1,1`), a row, a section (`s:2`, `s:body`), a bookmark, a text range, the
    same forms as the other Word tools. `expect: "one"` counts the matches in the range alone. A range
    that names nothing, or a reversed span, is refused as the block tools refuse it.
  - `regex` is Python's regular expression, as `find_text`'s: a pattern whose meaning here could differ
    from Python's `re` is refused with the call that finds the text as written. In `replace`, `\1` and
    `\g<name>` are the match's groups.
  - Tracked, each match is recorded as Word's own Find and Replace records one: the match deleted and
    the replacement inserted after it, the rest of the paragraph untouched (measured in Word 16.106).
    With `regex`, the match is deleted whole and its expansion inserted whole. Accept All then gives
    the text Word's own replace by wildcards gives untracked. Word 16.106 for Mac records a tracked
    wildcard replace piece by piece, and those pieces accepted do not read as the replacement.
- Still refused as not supported: an empty `find`, and styled text in `replace` (`**`, `*`,
  `[text]{Style}`, escapes).

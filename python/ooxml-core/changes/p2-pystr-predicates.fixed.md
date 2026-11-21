Lane P2e3: where the Rust core reads a character as Python does, it reads Python's own
predicate (`ooxml_base::pystr`, tables generated from CPython 3.12 by
`tools/pystr_tables.py`): automatic hyphenation's letters (`str.isalpha`, not Unicode's
Alphabetic: combining vowel signs and letter numbers are no letters) and the digits beside a
word (`str.isdigit`); a number format's `[ColorN]` (`isdigit`: a vulgar fraction is no
colour number); the bookmark-name check (`re`'s `\w` and `\d`); and which paragraphs' records
the Rust export makes (a paragraph with a non-ASCII digit stays Python's, one with another
numeric character no longer does).

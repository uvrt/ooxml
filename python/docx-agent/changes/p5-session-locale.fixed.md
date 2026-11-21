**The document's locale reaches the Rust session (lane P5q).** A served `word_fields` call
passes the document's `locale` in its `CallContext`, so a table of contents' `\t` styles are
read as `_list_separator` and `_field_language` read them (the locale's list separator and
language, else the document's own language) instead of from `w:themeFontLang` alone.

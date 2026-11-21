**A call's locale (lane P5q).** `CallContext(locale=...)` and `Session(locale=...)` take
ooxml-common's `Locale` (or a BCP 47 tag, read as `Locale(language=tag)`): a table of
contents' `\t` styles are split on its list separator and found in its language, in
docx-agent's order -- the call's locale, then the session's where the host set one, then
the document's own language (`w:themeFontLang`, then the default run language).
`Session()` without a locale is en-US as before and sets none for its calls. One locale type
across the crates: `ooxml_numfmt::Locale`, from which a context's tag
(`ooxml_base::Locale`) is derived.

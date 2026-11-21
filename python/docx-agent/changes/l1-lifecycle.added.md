Lane L1: `set_properties` is served by the Rust session (`ooxml_tools_word::SetProperties`,
the edit core's `SetCoreProperties`): the title, author, language and subject set in
`docProps/core.xml` as docx-agent writes them -- a new element where Word writes it, the part
made with its relationship and content type when the package has none, no step when every
value already is the one given -- and the four read back.

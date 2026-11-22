`docx2svg.rust.page_setup(options)` (lane H7b): what a Rust session lays its documents out
with for these options -- an `ooxml_core.PageSetup` of the fonts `word_fonts` finds, the
engines' locale, decimal tab and glyph size, counted in the process's memory budget -- and `engine_key(options)`, what an engine for them is made for -- so the
session keeps each document's engine itself; `None` for options the Rust layout does not take.

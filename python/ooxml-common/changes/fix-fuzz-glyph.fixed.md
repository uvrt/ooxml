A font whose `cmap` or `kern` ranges are laid over each other is refused in a moment (the Rust reader's `glyph_ink`
fuzz timeout; a document's embedded face or a font folder could ask for hours). A format 12/13 `cmap` subtable whose
groups claim more than the Unicode codespace (0x110000) in all, or a format 4 one whose segments claim more than
0x10000, is refused before any code is read (`fonts.office`, and `fonts.sfnt`, whose count of distinct codes never
stopped groups repeating the same ones); `fonts.office` skips a group wider than the codespace, as the Rust reader
does. `read_legacy_kern` reads no more pairs than the table's bytes hold and stops at an Apple subtable shorter than
its header. Fonts within the spec read as before (1,415 macOS and Office faces compared).

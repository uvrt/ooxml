The raster font filter (`fonts.rasterdb`) reads a face's coverage from the one subtable each of resvg's readers
selects, as they select it: the shaper's (harfrust: symbol, then full Unicode, then BMP, then Mac Roman, the first
record of a format it reads) and the fallback check's (skrifa: formats 4 and 12 only, symbol over full Unicode over
BMP, the last record of a kind) -- not every Unicode subtable's intersection and every subtable's union. A
character only another subtable maps is never drawn, and one subtable is read per reader, so a `cmap` of thousands
of subtables costs no more than one. A subtable refused as laid over itself counts as anything for the fallback (the
face is kept). Real faces: the shaper's coverage grows where a full-Unicode subtable maps more than the BMP one
(astral characters, emoji) or a symbol face had no Unicode subtable; the fallback's shrinks to what resvg's fallback
finds (Mac Roman codes no longer read as Unicode, LastResort's format 13). The raster set can be smaller.

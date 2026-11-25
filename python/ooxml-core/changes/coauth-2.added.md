A saved file's MS-FSSHTTPD 2.4.1 chunks for a co-authoring host (the design page "CyberAI as an Office Client",
request 4; lane coauth-2): `ooxml_core.chunk_file(data)` cuts and signs a file as Office does (an entry of at most
4,096 bytes one chunk signed SHA-1(header) XOR (CRC-32, sizes); a larger one a header and a data chunk; the central
directory the final chunk; a chunk over 1 MiB with random 1 MiB subchunks; a file Office does not analyse as ZIP in
1 MiB pieces) and gives `FileChunks`: its `chunks` (`FileChunk`: kind, entry name, offset, size, signature, random,
subchunks), `leaves()` as `(signature, size)`, the `root` and `delta(previous)`, a `ChunkDelta` naming the chunks a
host holding the previous version's leaves must be sent (`new`, `held`, `repeated`, `new_bytes`; an unchanged large
entry is reused through its parent leaf). The chunking is the shared Rust package layer's (`ooxml_zip::chunks`), so
docx, pptx and xlsx saves are signed in one place; written from the probe side's clean-room description and held to
its four test vectors byte for byte.

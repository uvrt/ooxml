Lane GS1 (tests): the golden transcripts pin a saved document by its parts, not its zip bytes.
`output.parts_sha256` (`tests/goldens_replay.py`, `package_digest`) hashes each part's name and
uncompressed bytes, by name; `save_document`'s `size` stays in the reply and in `data_keys` but
is out of `data_sha` (`ENCODING_FIELDS`). The same parts deflated by zlib (the Python save) and
zlib-rs (the Rust session's) differ in size, and the two write some parts in another order;
Word sees neither, so the goldens now hold on both paths. `tools/refresh_goldens.py` compares
the parts the same way.

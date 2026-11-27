- **The facade over the Rust core** (Phase 3 E2: `ooxml_core` is internal and reached only here):
  - `ooxml_edit.chunks`: `chunk_file`, `FileChunks`, `FileChunk` and `ChunkDelta`, a saved file's
    MS-FSSHTTPD chunks for a co-authoring host (what `ooxml_core.chunk_file` was).
  - `ooxml_edit.tools.hosted`: `rust_host`, `tool_specs`, `host_tools`, `host_held_kinds` and
    `open_refusal`, what a format library's `DocumentFormat.host` makes its host and definitions
    with.
  - `HostedSession.live_view(doc, since=None)` and `page_svg(doc, page, token, text="text")`: the
    live view of a Word document on the Rust host, laid out by its own engine within the session's
    limits; `hosted.UnknownToken` for a token no view gave or kept.

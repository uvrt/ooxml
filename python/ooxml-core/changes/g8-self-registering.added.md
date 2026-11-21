Lane G8: `ooxml_core.tool_hosting(kind)`: how a host hands each tool its calls, as JSON
(`{name: {"served", "blobs", "layout", "keeps_written"}}`), each Rust tool's
`ooxml_tools_word::Hosting`. The tools register themselves where they are defined
(`ooxml_tools_word::Registrar`, `register`); `Registry::standard` no longer lists them, and
`Call` and the handlers are in modules of their own (`call.rs`, `handlers.rs`). The
definitions (`tool_specs`) are unchanged, byte for byte.

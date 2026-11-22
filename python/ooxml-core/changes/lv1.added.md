- **The Word live view on the session (lane LV2).** `Session.view(doc, since=None, *, layout,
  context=None)` gives the head version's pages (each `(number, blocks, width, height)`: the
  ids of the body's top-level blocks it draws, in document order, its size in points), the
  pages whose drawing changed since an earlier view's token (every page without one; those
  the earlier version did not have too), and a token, `<session nonce>-<doc>-<state
  number>`; `stale` with a `note` where `since` names no version kept (another session's,
  or older than the last four). `Session.page_svg(doc, page, token, *, layout, text="text",
  context=None)` draws one page of the version a token names as SVG, its paragraph groups
  tagged with that version's ids (`data-docx-agent-id`, a header's, footer's or note's
  paragraph's from its part), even after the document moved on: a version the engine no
  longer paints is laid out again on a spare engine, never becoming the engine's last.
  Both lay out with `layout`'s page engine (`CallLayout(pages=)`) within the context's
  limit on laying out, with the interpreter lock released; a version the engine does not
  lay out yet, and `text="outlines"`, raise `ooxml_core.ViewRefused` (as `render` refuses);
  an unknown token `ooxml_core.UnknownToken`. In Rust: `ooxml_session::Session::view` and
  `page_svg` (`live_view`, the kept versions counted in the session's memory budget by
  their layouts' weight), `ooxml_tools_word::laid_to_draw` (render's laying out within the
  call's limit, shared), `ooxml_word_pages::LaidVersion::top_blocks`.

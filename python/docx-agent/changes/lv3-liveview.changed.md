- **The live view is the Rust session's (lane LV3).** `docx_agent.liveview.view(session,
  doc)` and `page_svg(session, doc, page, token)` keep their names and answers (`View`,
  `ViewPage`, `PageSvg`, `UnknownToken`) and call `ooxml_core.Session.view` / `page_svg`
  with the document's page engine: a token is `<session>-<doc>-<state number>`, the last
  four versions are kept within the session's memory budget, a page of an older version is
  drawn from its own layout. `ooxml_core.UnknownToken` is raised as `UnknownToken`, a
  version the session does not lay out yet (and `text="outlines"`, not drawn in Rust yet:
  the way out `text="text"`) as the session's refusal (`ToolError`). `externalise_images`
  and `embed_images` are unchanged. `view(document)` / `page_svg(document, ...)` on a
  document outside a session raise `TypeError` naming the way out (open it in a session).
  The worker tasks `view_bytes` / `page_bytes`, the token list kept in Python, the lock
  handling and their layouts kept with the shared layout (`extra["views"]`) are gone.

- **The module is internal** (decision D6, Phase 3 E2): only ooxml-edit's facade and docx2svg import
  it; applications and the format libraries use `ooxml_edit.tools` and `ooxml_edit.chunks`.
  Removed with the Python host of Word documents: `Session` (and `open_group`), `StepGroup`,
  `GroupMark`, `CallOptions`, `CallDocuments`, `CallLayout`, `CallWriting`, `CallContext`,
  `CancelToken` and `Limits`, the Python callbacks they carried (blobs, a placer, a host's
  tables), and the host's names for documents. `HostSession.view` and `page_svg` take the document,
  `since`, the page, the token and the text mode only: the document's own engine lays it out,
  within the host's limits.
  Instead: `ooxml_edit.tools` (the `Toolbox` and its sessions; `HostedSession.live_view` and
  `page_svg`), `ooxml_edit.tools.hosted` for a format library, and `ooxml_edit.chunks`.

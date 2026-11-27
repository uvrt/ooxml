- **`ooxml_core` is internal** (decision D6, Phase 3 E2). docx-agent, xlsx-agent and pptx-agent import
  nothing from it; applications use ooxml-edit's facade and the libraries' own APIs. The module keeps
  its name, as the engine ooxml-edit and docx2svg are built on; nothing outside them may rely on it.
  Instead: check's `problems` and each edit's `checks.validate` (in place of `ooxml_core` problems and
  `baseline_problems`); `chunk_file` through `ooxml_edit.chunks`. Every public name, with its
  replacement:
  - `Session` (`open`, `open_state`, `call`, `edit`, `save`, `close`, `undo`, `redo`, `validation`,
    `numbers`, `retired`, `documents`, `version`, `head`, `store_stats`, ...): gone. Instead:
    `Toolbox(TOOLS, formats=[FORMAT]).session()` and its `open`, `save`, `close`, `undo`, `redo`,
    with `toolbox.dispatch(session, tool, arguments)` for every tool; a document's problems are
    `check`'s `problems` (include `validate`) and each edit's `checks.validate`.
  - `Session.open_group`, `StepGroup`, `GroupMark`: gone. Instead: the `batch` tool.
  - `Session.view`, `Session.page_svg`, `CallLayout`: gone. Instead: `docx_agent.liveview.view`
    and `page_svg` (`HostedSession.live_view` and `page_svg` underneath); `UnknownToken` is
    `docx_agent.liveview.UnknownToken`, and `ViewRefused` is raised as the tool layer's `ToolError`.
  - `CallOptions`, `CallDocuments`, `CallWriting`, `CallContext`, `CancelToken`, `Limits`: gone.
    Instead: `Toolbox(limits=ooxml_edit.tools.Limits(...))` and a session view's `context`.
  - `Host`, `HostSession`, `OpenRefused`, `Version`: internal. Instead: the `Toolbox` and its
    sessions; a refused open raises the tool layer's `StateRefused` or `ArgumentError`;
    `docx_agent.tools.block_of` and `blocks_between` read a document's blocks.
  - `tool_specs`: internal. Instead: `toolbox.definitions(provider)`, or for a format library
    `ooxml_edit.tools.hosted.tool_specs(kind)`.
  - `new_document`: internal. Instead: the `new_document` tool.
  - `chunk_file`, `FileChunks`, `FileChunk`, `ChunkDelta`: internal. Instead: `ooxml_edit.chunks`.
  - `WordFonts`, `WordPages`, `LaidPages`, `LayoutSummary`, `PageSetup`, `PageRaster`,
    `MemoryBudget`, `BudgetAccount`, `drawing_plan`, `element_path`: internal (docx2svg's engine).
    Instead: docx2svg's own API, and `render` and `check` for a session's document.
  - `StateRefused`, `NotWordPackage`: internal. Instead: `ooxml_edit.tools.StateRefused` and the
    open's `ArgumentError`, which the Toolbox raises.

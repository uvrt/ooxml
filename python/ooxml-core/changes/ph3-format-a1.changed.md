- The session is format-generic underneath (architecture review Phase 3, lane ph3-format, PR A1): each open document
  is a format's `FormatDoc` over one generic history (`Stored`: undo and redo, a caller's own undo, step owners, state
  numbers, step groups), and a version is held erased (`AnyVersion`). The tools register through
  `ooxml_tools::Registrar` of their format's types and run as `ooxml_tools::Tool` in a `CallEnv`. Tool results
  unchanged.
- `ooxml_core.tool_hosting` is gone with the Rust tools' `Hosting` flags: docx-agent lists the few tools that read
  blobs, pages or values written by reference itself, until the Rust host hands every call what it reads.

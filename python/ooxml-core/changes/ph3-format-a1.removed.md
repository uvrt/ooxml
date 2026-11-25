`ooxml_core.tool_hosting` is gone (lane ph3-format, PR A1), with the Rust tools' `Hosting` flags (`blobs`,
`layout`, `keeps_written`): the Rust host hands every call what it reads, so no tool declares it. docx-agent lists
the few tools that read blobs, pages or values written by reference itself until its Python host goes.
Instead: nothing replaces it; every Word tool is the session's (read `served` as true for every tool
`ooxml_core.tool_specs("docx")` lists).

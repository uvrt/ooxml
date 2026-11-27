- **The Python host of Word documents is gone** (Phase 3 E2). Every call of a Word tool runs on
  the Rust host through ooxml-edit's `Toolbox`, as it has by default since the Rust host's switch;
  nothing could route one elsewhere but a test seam.
  - `docx_agent.rust` is removed: the owner and its process-wide session (`RustOwner`, `OWNER`),
    the document's mirror, the served adapters, `RustTools` and `RustResult`. Instead: the
    `Toolbox`, `Session` and `Result` of `ooxml_edit.tools` with docx-agent's `TOOLS` and `FORMAT`.
  - `TemplateOpened` is `docx_agent.tools.TemplateOpened` (it was `docx_agent.rust.owner`'s).
  - docx-agent's `FORMAT` keeps its definitions, prompt, strict set, tool hints, withdrawn tools
    and its host. Its Python hooks (`summary`, `problems`, `validation`, `checks`, `state`,
    `own_source`, `source_hint`, `own_facts`, `number_texts`, `scale_values`, `read_provenance`,
    `owner`, `serve`) are gone: the Rust host answers each.
  - `block_of` and `blocks_between` take `(session, doc, ...)` only. Their in-handler form
    `(call, ...)` served docx-agent's own Python handlers, which are gone.
- **A toolbox holding docx-agent's format beside a Python-held format of a kind the Rust session
  cannot name is refused when it is made** (`ValueError`). The message names the kinds the Rust
  session can name (`xlsx`, `pptx`) and the way out: give that format a toolbox of its own. Before,
  such a toolbox kept its Word documents on the Python host.

- **The tool layer's owned documents are gone** (Phase 3 E2): `DocumentFormat.owner` and
  `DocumentFormat.serve`, `ooxml_edit.tools.OwnedDocument`, and the modules `ooxml_edit.tools.owned`,
  `served` and `gate`. They held docx-agent's Word documents on the Python host, which is gone.
  The Python host keeps the pptx and xlsx documents until Phase 4.
  Instead: `DocumentFormat.host`, a format whose documents the Rust host holds and whose tools it
  runs (`ooxml_edit.tools.hosted`).
- **A toolbox holding a format the Rust host holds beside a Python-held format of a kind the Rust
  session cannot name is refused when it is made** (`ArgumentError`, a `ValueError`), naming the
  kinds it can name. Before, such a toolbox ran on the Python host alone.

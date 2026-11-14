# ooxml

Reading, laying out, rendering and editing Office Open XML documents (Word, PowerPoint,
Excel), in one repository.

The packages keep their distribution and import names. Each lives under `python/` with
its own `pyproject.toml`, version, changelog and tests:

| Distribution | Import | What it is |
|---|---|---|
| ooxml-common | `ooxml_common` | shared OOXML reading, units, fonts and text measuring |
| ooxml-edit | `ooxml_edit` | the editing layer the agents are built on |
| docx2svg | `docx2svg` | Word layout and rendering to SVG/PNG |
| pptx2svg | `pptx2svg` | PowerPoint rendering to SVG/PNG |
| docx-agent | `docx_agent` | tools for editing Word documents |
| pptx-agent | `pptx_agent` | tools for editing PowerPoint decks |
| xlsx-agent | `xlsx_agent` | tools for editing Excel workbooks |

Each package's own history is kept: it was brought in with a merge commit from its former
repository, so `git log -- python/<package>` and `git blame` reach back to its first commit.

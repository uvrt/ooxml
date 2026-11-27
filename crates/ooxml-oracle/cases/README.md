# Frozen cases

The differential gates' cases, frozen as data (architecture review decision 2, R4 #2). The gates
run these cases through the Rust oracle binaries and compare each answer with its golden in
`../goldens`. Nothing here is computed at gate time. `crates/ooxml-oracle/python/goldens.py`
is the reference for this format.

## Layout

- `fixtures/<document>.jsonl`: every case of every mode on one fixture document.
  - `<document>` is the document's path below `crates/ooxml-oracle/fixtures`, for example
    `fixtures/generated/controls.docx.jsonl`.
  - The edit modes run on these documents in the `fixtures` gate, the read modes in the `reads`
    gate.
- `corpus/<document>.jsonl`: the same for the frozen corpus (`crates/ooxml-oracle/corpus`).
- `<tree>/_made.jsonl`: cases on documents no tree holds. These are made-up documents, such as
  the facts check's.
- `inputs/<digest>.docx`: documents a case runs on instead of its own, named by the digest of
  their bytes. Examples are a French copy (spacing), a document reopened with its id state
  (ids) and a made-up document.

## A line

One JSON object per line, with sorted keys and no spaces:

```json
{"case":{...},"id":"structure:0123456789abcdef","mode":"structure"}
```

- `mode` is a `tools/gates.py` mode: an edit mode (`set_text` … `style`) or a read mode
  (`ids` … `laid`).
- `case` is what the oracle binary is given.
  - For an edit mode, this is the case dict as the case makers built it (`op`, `target`, `text`,
    `tracked`, `tool`, `arguments`, `reference_file`, …).
  - For a read mode, this is the manifest item less its `file` (`case`, `tool`, `arguments`,
    `fonts`, …).
  - `input` names the frozen document the case runs on, when that is not its own.
- `id` is `<mode>:<the first 16 hex digits of the SHA-256 of the canonical case JSON>`. It is
  the same whatever a run's sampling, and the goldens are keyed by it.
- Paths are portable:
  - `$ROOT/...` is the repository.
  - `$INPUT/...` is `inputs/`.
  - `$HOME/...` is the home folder; the runners keep their substitute fonts there.

A file's cases are in the order the case makers made them. `--per-tool N` samples the first N
cases of each tool (the tool modes: structure, objects, style), or else the first N cases of
the document.

## What was frozen

- **Fixtures.** Every case the case makers made on every fixture, all modes.
- **Corpus, capped.** The first 10 cases per tool (tool modes) or per document (the other
  modes). That is the coverage the corpus gate had before goldens (`CORPUS_PER_TOOL`).

The cases were made by the case makers in `crates/ooxml-oracle/python` against the frozen
reference (docx-agent at the commit `../reference.toml` names). A new case is written here as
data, and blessed (`python tools/gates.py bless`) with the source of its answer.

## The stamp-on-open re-target

`stamp-on-open-renamed.jsonl` maps each case id the stamp-on-open change retired (decision
2026-11-26) to the case that replaced it: `{"document", "new", "old", "tree"}`. Lane ph3-st1
made it with `python/stamp_on_open.py` from each document's ids before and after the stamp
at open. That tool, and the oracle's `word-ids --renamed` it read, were removed in ST2, when
names by place stopped being given at all. The map is kept as data.

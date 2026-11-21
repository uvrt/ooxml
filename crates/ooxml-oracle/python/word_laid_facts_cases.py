"""``check``'s layout facts read from the pages, docx-agent against Rust (test support;
lane P5f): ``word_reads_diff.py laid``.

For each document, docx-agent's ``layout_facts`` of its layout -- the facts the Rust tools
give so far (:data:`KEYS`, ``settings``' :data:`SETTINGS`), capped as ``check`` lists them
-- once without a minimum text size and once with each of :data:`MINIMUMS`, and for some
ranges with each range's report (``range``: extent, keeps, each keep tried), against the
``word-reads`` binary's laid fact cases (``crates/ooxml-oracle/src/laid_facts.rs``: a page
engine laying the document out with docx-agent's fonts, ``fonts_of``).

Python's behaviour is not changed: this only calls docx-agent's documented functions.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

TESTS = Path(__file__).resolve().parents[3] / "python" / "docx-agent" / "tests"
if str(TESTS) not in sys.path:
    sys.path.insert(0, str(TESTS))

from docx_agent import Document
from docx_agent.layout_facts import layout_facts
from docx_agent.scope import Scope

#: The facts compared, with their ``_left_out`` counts.
KEYS = ("smallest_text", "small_text", "spacers", "keep_with_next_at_foot", "table_parted", "split_rows",
        "header_not_repeated", "column_fit", "past_margins", "uncertain", "unknown_from")
#: ``settings``' facts compared.
SETTINGS = ("keep_together",)
#: The minimum text sizes tried besides none: one number, and one per kind with a caption's.
MINIMUMS = {"min 9": 9, "min by kind": {"body": 10.5, "table": 9, "caption": 11, "header": 8}}


def _picked(found: dict) -> dict:
    out = {k: v for k, v in found.items() if k.removesuffix("_left_out") in KEYS}
    settings = found.get("settings") or {}
    out["settings"] = {k: v for k, v in settings.items() if k.removesuffix("_left_out") in SETTINGS}
    return out


def _ranges(document) -> list[str]:
    """The range tried: the body's first table, else its middle top-level block."""
    from docx_agent.layout_facts import _W

    main = document.package.document_part()
    body = document.package.tree(main).find(_W + "body")
    if body is None:
        return []
    index = document._index(main)
    blocks = [index.entry_for(child) for child in body if child.tag in (_W + "p", _W + "tbl")]
    blocks = [entry for entry in blocks if entry is not None]
    tables = [entry.id for entry in blocks if entry.id.startswith("t")]
    if tables:
        return [tables[0]]
    return [blocks[len(blocks) // 2].id] if blocks else []


def _block_ranges(document, layout) -> dict[str, list[str]]:
    """The ranges tried besides :func:`_ranges`' (each range's keep, ``check``'s ``range``
    report): ``"break"``, the span of the two top-level blocks either side of the first page
    break (its keep tried, where it can close the break); ``"chain"``, the blocks page 2
    starts with that keep with the next, and the one they keep with (a kept range starting
    a page: its release tried)."""
    from docx_agent.edit import effective
    from docx_agent.layout_facts import _W

    out: dict[str, list[str]] = {}
    first, second = layout.blocks_on(1), layout.blocks_on(2)
    if len(first) >= 2 and second:
        before = first[-1] if first[-1] not in second else first[-2]
        out["break"] = [f"{before}..{second[0]}"] if before != second[0] else []
    if second:
        main = document.package.document_part()
        body = document.package.tree(main).find(_W + "body")
        index = document._index(main)
        blocks = [(entry.id, child) for child in body if child.tag in (_W + "p", _W + "tbl")
                  for entry in [index.entry_for(child)] if entry is not None]
        ids = [identifier for identifier, _ in blocks]
        if second[0] in ids:
            start = end = ids.index(second[0])

            def chained(element) -> bool:
                inside = [element] if element.tag == _W + "p" else list(element.iter(_W + "p"))
                return all(effective.paragraph(document, paragraph).keep_with_next for paragraph in inside)

            while end + 1 < len(blocks) and chained(blocks[end][1]):
                end += 1
            out["chain"] = [f"{ids[start]}..{ids[end]}" if end > start else ids[start]]
    return {name: ranges for name, ranges in out.items() if ranges}


def _range_report(document, layout, scope) -> list:
    """Each range as ``check`` reports it -- its extent and keeps -- with each keep tried
    from this layout (``keep_trials.keep_effects``, its caps)."""
    from docx_agent.layout import layout_options
    from docx_agent.tools.keep_trials import keep_effects
    from docx_agent.tools.scoped import range_pages

    reported, _ = range_pages(scope, layout, extent=True)
    keep_effects(layout._raw, document.to_bytes(), layout_options(document, {}), layout, scope, reported)
    return reported


def check_laid(args, run_rust, report, files_of) -> int:
    from word_structure_cases import fonts_of

    expected, manifest = {}, []
    for path in files_of(args):
        try:
            document = Document.open(path.read_bytes())
            layout = document.layout()
            fonts = fonts_of(document)
        except Exception as error:  # noqa: BLE001 -- a test instrument: say what failed
            print(f"{path}: python failed to lay out: {error}")
            continue
        for case, minimum in {"none": None, **MINIMUMS}.items():
            try:
                found = json.loads(json.dumps(_picked(layout_facts(layout, min_size=minimum))))
            except Exception as error:  # noqa: BLE001
                print(f"{path} [{case}]: python failed: {error}")
                continue
            expected[(str(path), case)] = {"facts": found}
            item = {"file": str(path), "case": case, "laid": True, "fonts": fonts}
            if minimum is not None:
                item["min_size"] = minimum
            manifest.append(item)
        # The ranges: the body's first table, else its middle top-level block (check's
        # range: the facts about its blocks, on its pages, and each range's report), and
        # the spans a keep is tried for (:func:`_block_ranges`).
        cases = {"range": _ranges(document)}
        try:
            cases.update(_block_ranges(document, layout))
        except Exception as error:  # noqa: BLE001
            print(f"{path} [ranges]: python failed: {error}")
        for case, ranges in cases.items():
            if not ranges:
                continue
            try:
                scope = Scope(document, ranges)
                pages = sorted({p for found in scope.pages(layout) for p in found})
                found = _picked(layout_facts(layout, pages=pages, scope=scope))
                found["range"] = _range_report(document, layout, scope)
                found = json.loads(json.dumps(found))
            except Exception as error:  # noqa: BLE001
                print(f"{path} [{case}]: python failed: {error}")
                continue
            expected[(str(path), case)] = {"facts": found}
            manifest.append({"file": str(path), "case": case, "laid": True, "fonts": fonts, "range": ranges})
    rust = {key: line["reply"] for key, line in run_rust(args.rust, manifest).items()}
    return report(expected, rust)

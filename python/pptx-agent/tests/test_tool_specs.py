"""The tool definitions a model is sent, one JSON per tool in tests/specs/ (decision 6,
R4 #11a): every tool or argument change shows in the pull request's diff, and
tools/release_notes.py compares them between release tags.  After an intended change,
write them again: ``UPDATE_SPECS=1 python -m pytest tests/test_tool_specs.py``."""

from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

from ooxml_edit.tools import Toolbox

from pptx_agent.tools import FORMAT, GROUPS, TOOLS

SPECS = Path(__file__).resolve().parent / "specs"
#: What the provider's request carries beside the definition (prompt caching).
TRANSPORT = ("cache_control",)


def definitions() -> dict[str, dict[str, Any]]:
    """Every tool's Claude definition, all groups, none deferred."""
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        sent = toolbox.definitions("anthropic", defer=False)
    return {d["name"]: {k: v for k, v in d.items() if k not in TRANSPORT} for d in sent}


def text(definition: dict[str, Any]) -> str:
    return json.dumps(definition, indent=1, sort_keys=True, ensure_ascii=False) + "\n"


def test_the_definitions_are_the_committed_specs() -> None:
    found = definitions()
    if os.environ.get("UPDATE_SPECS"):
        SPECS.mkdir(exist_ok=True)
        for path in SPECS.glob("*.json"):
            if path.stem not in found:
                path.unlink()
        for name, definition in found.items():
            (SPECS / f"{name}.json").write_text(text(definition), encoding="utf-8")
    assert sorted(p.stem for p in SPECS.glob("*.json")) == sorted(found), "UPDATE_SPECS=1 writes them"
    for name, definition in found.items():
        assert (SPECS / f"{name}.json").read_text(encoding="utf-8") == text(definition), name

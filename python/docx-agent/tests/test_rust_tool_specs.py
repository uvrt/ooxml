"""The Rust tool contract's generated definitions against today's Python ones.

``crates/ooxml-tools`` builds a tool's schema, validation schema and declarations from one
Rust struct (``#[derive(ToolArgs)]``).  Three of today's tools are ported there as examples
(``crates/ooxml-tools/tests/ported``), and their exports are kept in
``crates/ooxml-tools/tests/golden``; the Rust test ``tests/integration/equality.rs`` holds the generated
exports to those files byte for byte.  This test holds the same files to today's Python
definitions byte for byte, and runs today's checker and adapters on them unchanged: a
``Tool`` made from a golden passes ``definition_problems`` (``subset_problems`` with the
bound keywords), and its canonical schema passes ``subset_problems`` and the Anthropic,
OpenAI Responses and Chat Completions adapters' strict rules.
"""

from __future__ import annotations

import copy
import json
from pathlib import Path

import pytest

from ooxml_edit.tools import Tool
from ooxml_edit.tools.adapters import (anthropic_problems, openai_problems, openai_strict_schema,
                                       to_anthropic, to_openai_chat, to_openai_responses)
from ooxml_edit.tools.schema import subset_problems

from docx_agent.tools import TOOLS

GOLDEN = Path(__file__).resolve().parents[3] / "crates" / "ooxml-tools" / "tests" / "golden"
PORTED = ("word_set_text", "word_format", "replace_text")


def export(tool: Tool) -> dict:
    """What ``ToolSpec::export`` writes, from today's Python definition: copies of its
    schemas, so that a test changing the export leaves the shared tool as it is (``undo``'s
    ``scope`` popped from the tool's own schema refused it in every later call of the
    worker)."""
    strings = list
    return {
        "name": tool.name, "description": tool.description, "group": tool.group,
        "mutates": tool.mutates, "strict": tool.strict, "batchable": tool.batchable,
        "read_batchable": tool.read_batchable, "retry_key": tool.retry_key, "role": tool.role,
        "priority": tool.priority, "route": tool.route, "documents": strings(tool.documents),
        "exactly_one": [strings(group) for group in tool.exactly_one],
        "refs": strings(tool.refs), "datasets": strings(tool.datasets),
        "sources": strings(tool.sources), "facts": strings(tool.facts),
        "aliases": dict(tool.aliases), "defaults": dict(tool.defaults),
        "kind_fields": {name: strings(kinds) for name, kinds in tool.kind_fields.items()},
        "kind_neutral": {name: strings(values) for name, values in tool.kind_neutral.items()},
        "kind_values": {name: {value: strings(kinds) for value, kinds in values.items()}
                        for name, values in tool.kind_values.items()},
        "kind_descriptions": {name: dict(texts) for name, texts in tool.kind_descriptions.items()},
        "schema": copy.deepcopy(tool.schema), "canonical": tool.canonical,
    }


def text(data: dict) -> str:
    return json.dumps(data, ensure_ascii=False, separators=(",", ":")) + "\n"


def from_golden(data: dict) -> Tool:
    """A Python ``Tool`` made from a golden export, serving Word documents."""
    return Tool(name=data["name"], description=data["description"], schema=data["schema"],
                handlers={"docx": lambda call, **arguments: None}, group=data["group"],
                mutates=data["mutates"], strict=data["strict"], batchable=data["batchable"],
                read_batchable=data["read_batchable"], retry_key=data["retry_key"],
                role=data["role"], priority=data["priority"], route=data["route"],
                documents=tuple(data["documents"]),
                exactly_one=tuple(tuple(group) for group in data["exactly_one"]),
                refs=tuple(data["refs"]), datasets=tuple(data["datasets"]),
                sources=tuple(data["sources"]), facts=tuple(data["facts"]),
                aliases=data["aliases"], defaults=data["defaults"],
                kind_fields={k: tuple(v) for k, v in data["kind_fields"].items()},
                kind_neutral={k: tuple(v) for k, v in data["kind_neutral"].items()},
                kind_values={k: {v: tuple(kinds) for v, kinds in values.items()}
                             for k, values in data["kind_values"].items()},
                kind_descriptions=data["kind_descriptions"])


@pytest.mark.parametrize("name", PORTED)
def test_the_rust_export_equals_todays_definition_byte_for_byte(name):
    tool = {t.name: t for t in TOOLS}[name]
    assert (GOLDEN / f"{name}.json").read_text(encoding="utf-8") == text(export(tool))


@pytest.mark.parametrize("name", PORTED)
def test_the_rust_schemas_pass_todays_checker_and_adapters(name):
    data = json.loads((GOLDEN / f"{name}.json").read_text(encoding="utf-8"))
    assert subset_problems(data["schema"], allow_bounds=True) == []
    assert subset_problems(data["canonical"]) == []
    tool = from_golden(data)          # definition_problems, in Tool.__post_init__
    assert tool.canonical == data["canonical"]
    assert anthropic_problems(to_anthropic([tool])) == []
    assert openai_problems(to_openai_responses([tool])) == []
    assert openai_problems(to_openai_chat([tool]), chat=True) == []
    assert openai_strict_schema(data["canonical"], data["schema"])["additionalProperties"] is False


# -- the intent rules and the validator, call by call ------------------------------------------
#
# ``crates/ooxml-tools/tests/golden/calls.json`` holds calls of the ported tools and what
# today's dispatcher makes of them before a handler runs: the arguments as meant (aliases,
# defaults, JSON strings, validation, exactly-one groups, then the kind rules for the call's
# document kind) and the notes, or the refusal (code, message, field, valid options).  This
# test holds the file to today's Python; ``crates/ooxml-tools/tests/integration/calls.rs`` holds the Rust
# contract to the same file.  The Rust refusals also carry a ``try_call`` (rule 2), which
# today's do not: that is outside what is compared.


def outcome(tool: Tool, arguments, kind: str | None) -> dict:
    """What ``Toolbox._arguments`` and ``Tool.ignored_for`` make of a call."""
    from ooxml_edit.tools.dispatch import _aliased, _defaulted, _json_strings
    from ooxml_edit.tools.results import ToolError
    from ooxml_edit.tools.schema import CallError, validate_call

    notes: list[str] = []
    try:
        if isinstance(arguments, str):
            arguments = json.loads(arguments or "{}")
        if arguments is None:
            arguments = {}
        arguments = _defaulted(tool, _aliased(tool, arguments, notes), notes)
        clean = validate_call(tool.schema, _json_strings(tool.schema, arguments), notes)
        for group in tool.exactly_one:
            given = [name for name in group if name in clean]
            if len(given) != 1:
                raise ToolError("invalid_arguments", f"give exactly one of {', '.join(group)}"
                                + (f"; got {', '.join(given)}" if given else ""),
                                field=given[1] if len(given) > 1 else group[0],
                                valid_options=list(group))
    except CallError as exc:
        return {"error": {"code": "invalid_arguments", "message": exc.message, "field": exc.field,
                          "valid_options": exc.valid_options}}
    except ToolError as exc:
        return {"error": {"code": exc.code, "message": exc.message, "field": exc.field,
                          "valid_options": exc.valid_options}}
    ignored = tool.ignored_for(kind, clean)
    return {"arguments": clean, "notes": notes + ignored}


def test_the_calls_golden_is_todays_dispatcher():
    tools = {t.name: t for t in TOOLS}
    cases = json.loads((GOLDEN / "calls.json").read_text(encoding="utf-8"))
    assert len(cases) >= 30
    for case in cases:
        got = outcome(tools[case["tool"]], case["arguments"], case["kind"])
        assert got == case["outcome"], case

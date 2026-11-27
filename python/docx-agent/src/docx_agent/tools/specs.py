"""The Word tools' definitions: each a :class:`~ooxml_edit.tools.Tool` made from the Rust
host's spec of it (``tool_specs``' exports, :func:`tool_of`).  Every call of a Word tool runs on
the Rust host (:mod:`ooxml_edit.tools.hosted`); the tool layer holds only its definition, which
the provider adapters (:mod:`ooxml_edit.tools.adapters`) send.
"""
from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any

from ooxml_edit.tools import Tool
from ooxml_edit.tools.results import Result, ToolError

KIND = "docx"


def on_host(call: Any, **arguments: Any) -> Result:
    """The handler of every Word tool: never reached -- its calls run on the Rust host."""
    raise ToolError("internal", f"{call.tool.name} runs on the Rust host, never in the tool layer")


def aliases_of(tool: str, exported: Mapping[str, Any], parameters: Sequence[str] = ()) -> dict[str, Any]:
    """A Rust tool's exported ``aliases`` as :attr:`Tool.aliases` takes them: a name stays a
    name; ``{"name", "when": {argument: value}}`` -- read only while an argument has a value
    -- is an :class:`~ooxml_edit.tools.AliasWhen`; ``names:<param>`` (a list of names given for a parameter that takes none can only
    mean ``param``) and ``place:<kind>`` (a place said in another placing tool's words) are
    the functions today's Python tools have for them (``names_as``, :func:`.positions.alias`);
    ``refused:<param>`` a name refused as unknown, naming ``param`` (:func:`refused_as`; the
    options are the tool's ``parameters``)."""
    from ooxml_edit.tools.registry import AliasWhen, names_as

    from .positions import alias

    out: dict[str, Any] = {}
    for key, target in exported.items():
        where, _, name = key.rpartition(".")
        if isinstance(target, Mapping):
            out[key] = AliasWhen.of_export(target)
        elif target.startswith("names:"):
            out[key] = names_as(name, target.removeprefix("names:"))
        elif target.startswith("place:"):
            out[key] = alias(tool, target.removeprefix("place:"), name, f"{where}." if where else "")
        elif target.startswith("refused:"):
            out[key] = refused_as(key, target.removeprefix("refused:"), parameters)
        else:
            out[key] = target
    return out


def refused_as(key: str, meant: str, parameters: Sequence[str]) -> Any:
    """The alias of a name that is not read as ``meant`` (Rust's ``refused_alias``): refused
    as an unknown argument, the refusal naming ``meant`` -- as today's ``markdown_from``."""

    def refuse(node: Mapping[str, Any]) -> None:
        raise ToolError("invalid_arguments", f"unknown argument {key!r}; did you mean {meant!r}?", field=key,
                        valid_options=sorted(parameters))

    return refuse


def tool_of(export: Mapping[str, Any], kind: str = KIND) -> Tool:
    """A :class:`Tool` from one of the host's tool specs (``tool_specs(kind)``' exports): its
    definition only (its calls run on the Rust host)."""
    return Tool(name=export["name"], description=export["description"], schema=export["schema"],
                handlers={kind: on_host}, group=export["group"], mutates=export["mutates"],
                strict=export["strict"], batchable=export["batchable"],
                read_batchable=export["read_batchable"], retry_key=export["retry_key"],
                role=export["role"], priority=export["priority"], route=export["route"],
                documents=tuple(export["documents"]),
                exactly_one=tuple(tuple(group) for group in export["exactly_one"]),
                refs=tuple(export["refs"]), datasets=tuple(export["datasets"]),
                sources=tuple(export["sources"]), facts=tuple(export["facts"]),
                aliases=aliases_of(export["name"], export["aliases"],
                                   list(export["schema"].get("properties", {}))), defaults=export["defaults"],
                kind_fields={k: tuple(v) for k, v in export["kind_fields"].items()},
                kind_neutral={k: tuple(v) for k, v in export["kind_neutral"].items()},
                kind_values={k: {v: tuple(kinds) for v, kinds in values.items()}
                             for k, values in export["kind_values"].items()},
                kind_descriptions=export["kind_descriptions"])

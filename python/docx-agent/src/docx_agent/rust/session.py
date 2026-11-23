"""Word tools of the Rust session as an application holds them itself.

:class:`RustTools` holds an :class:`ooxml_core.Session`: documents opened from their bytes
and tool calls run there with the interpreter lock let go (:meth:`RustTools.dispatch`).  The
tools' definitions come from Rust (``ooxml_core.tool_specs``) as
:class:`~ooxml_edit.tools.Tool` values (:func:`tool_of`, which docx-agent's toolbox defines
its tools by too), and their results as :class:`RustResult`, so the provider adapters
(:mod:`ooxml_edit.tools.adapters`) render them for Claude and OpenAI.
"""
from __future__ import annotations

import json
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass, field
from typing import Any

from ooxml_edit.tools import Tool
from ooxml_edit.tools.adapters import definitions_for, results_for
from ooxml_edit.tools.results import Result, ToolError

KIND = "docx"

def _runs_in_rust(call: Any, **arguments: Any) -> Result:
    raise ToolError("internal", "this tool runs in the Rust session: RustTools.dispatch")


def aliases_of(tool: str, exported: Mapping[str, Any], parameters: Sequence[str] = ()) -> dict[str, Any]:
    """A Rust tool's exported ``aliases`` as :attr:`Tool.aliases` takes them: a name stays a
    name; ``{"name", "when": {argument: value}}`` -- read only while an argument has a value
    -- is an :class:`~ooxml_edit.tools.AliasWhen`; ``names:<param>`` (a list of names given for a parameter that takes none can only
    mean ``param``) and ``place:<kind>`` (a place said in another placing tool's words) are
    the functions today's Python tools have for them (``names_as``, :func:`.positions.alias`);
    ``refused:<param>`` a name refused as unknown, naming ``param`` (:func:`refused_as`; the
    options are the tool's ``parameters``)."""
    from ooxml_edit.tools.registry import AliasWhen, names_as

    from ..tools.positions import alias

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
    """A :class:`Tool` from one of ``ooxml_core.tool_specs(kind)``'s exports: its
    definition only (the handler runs in Rust)."""
    return Tool(name=export["name"], description=export["description"], schema=export["schema"],
                handlers={kind: _runs_in_rust}, group=export["group"], mutates=export["mutates"],
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


@dataclass
class RustResult(Result):
    """A Rust call's reply as a :class:`Result`: :meth:`to_json` is the envelope Rust
    wrote, key for key (``try_call`` included), so
    :meth:`Result.to_text` and the adapters send exactly it."""

    envelope: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def of(cls, text: str, tool: str) -> "RustResult":
        envelope = json.loads(text)
        error = envelope.get("error")
        fields: dict[str, Any] = {key: envelope[key] for key in (
            "doc", "version", "summary", "changed", "created", "removed", "renamed", "warnings",
            "checks", "data", "next_cursor", "refs", "datasets", "total") if key in envelope}
        if error is not None:
            fields["error"] = ToolError(error["code"], error["message"], field=error.get("field"),
                                        valid_options=error.get("valid_options"),
                                        details=error.get("details"), try_call=error.get("try_call"))
            fields["summary"] = error["message"]
        return cls(ok=bool(envelope["ok"]), tool=tool, envelope=envelope, **fields)

    def to_json(self) -> dict[str, Any]:
        return json.loads(json.dumps(self.envelope))


class RustTools:
    """Word documents and tools in a Rust session.  Thread-safe: every call lets go of the
    interpreter lock for its Rust work."""

    def __init__(self, session: Any = None, *, kind: str = KIND) -> None:
        import ooxml_core

        self.session = session if session is not None else ooxml_core.Session()
        self.kind = kind
        self.tools = [tool_of(export, kind) for export in json.loads(self.session.tool_specs(kind))]

    def open(self, data: bytes, name: str) -> str:
        """Open a document's bytes; its id (``d1``)."""
        return self.session.open(data, kind=self.kind, name=name)

    def dispatch(self, name: str, arguments: Mapping[str, Any] | str, *,
                 context: Any = None) -> RustResult:
        """Run the tool ``name``; its reply as a :class:`RustResult`."""
        text = arguments if isinstance(arguments, str) else json.dumps(arguments)
        import ooxml_core

        reply, _images, _app = self.session.call(name, text, ooxml_core.CallOptions(context=context))
        return RustResult.of(reply, name)

    def definitions(self, provider: str, **options: Any) -> list[dict[str, Any]]:
        """The tools' definitions for ``provider`` (``anthropic``, ``openai-responses``,
        ``openai-chat``), by today's adapters."""
        return definitions_for(provider, self.tools, **options)

    @staticmethod
    def results(provider: str, results: Iterable[tuple[str, Result]], **options: Any) -> Any:
        """What answers one assistant turn for ``provider``, by today's adapters."""
        return results_for(provider, list(results), **options)

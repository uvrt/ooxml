"""Word tools served by the Rust session (lane P0d), for the switch-over that routes
docx-agent's reading tools through Rust (phase PL, step L4).

:class:`RustTools` holds an :class:`ooxml_core.Session`: documents opened from their bytes,
each later version's touched parts handed over (:meth:`RustTools.update`: the bridge's way
in, docs/architecture/README.md section 13), and tool calls run there with the interpreter
lock let go (:meth:`RustTools.dispatch`).  The tools' definitions come from Rust
(``ooxml_core.tool_specs``) as :class:`~ooxml_edit.tools.Tool` values, and their results as
:class:`RustResult`, so the provider adapters (:mod:`ooxml_edit.tools.adapters`) render both
unchanged for Claude and OpenAI.

The Rust session serves the reading tools so far (phase P1): ``describe`` with every field of
today's but ``pages`` and ``problems`` (``RUST_DESCRIBED``; ``tests/test_rust_describe.py``
holds them equal to today's on every fixture), ``word_read``, ``word_inspect`` (``layout``
with a placer the host gives), ``find_text`` (a regular expression where its meaning is
certainly Python's) and ``read_batch`` over them, equal to today's on every fixture but
drawings' chart and SmartArt summaries (``tests/test_rust_word_read.py``,
``tests/test_rust_reads.py``).  A version read once is kept for the calls after it, and a
new version (:meth:`RustTools.update`) reads only the story parts that changed.  docx-agent's
own toolbox answers its reading tools through the same session by default
(:mod:`docx_agent.rust.reads`, lane P1c); this module is the session's tools as an
application would hold them itself.
"""
from __future__ import annotations

import json
from collections.abc import Iterable, Mapping
from dataclasses import dataclass, field
from typing import Any

from ooxml_edit.tools import Tool
from ooxml_edit.tools.adapters import definitions_for, results_for
from ooxml_edit.tools.results import Result, ToolError

KIND = "docx"

#: The keys of ``describe``'s data the Rust session gives, in their order there (``tagged``
#: and ``headings_note`` only where today's has them).
RUST_DESCRIBED = ("name", "compatibility_mode", "properties", "tracking", "headings", "sections", "styles_in_use",
                  "comments", "revisions", "fields", "tables", "drawings", "notes", "controls", "tagged",
                  "headings_note")


def _runs_in_rust(call: Any, **arguments: Any) -> Result:
    raise ToolError("internal", "this tool runs in the Rust session: RustTools.dispatch")


def aliases_of(tool: str, exported: Mapping[str, str]) -> dict[str, Any]:
    """A Rust tool's exported ``aliases`` as :attr:`Tool.aliases` takes them: a name stays a
    name; ``names:<param>`` (a list of names given for a parameter that takes none can only
    mean ``param``) and ``place:<kind>`` (a place said in another placing tool's words) are
    the functions today's Python tools have for them (``names_as``, :func:`.positions.alias`)."""
    from ooxml_edit.tools.registry import names_as

    from ..tools.positions import alias

    out: dict[str, Any] = {}
    for key, target in exported.items():
        where, _, name = key.rpartition(".")
        if target.startswith("names:"):
            out[key] = names_as(name, target.removeprefix("names:"))
        elif target.startswith("place:"):
            out[key] = alias(tool, target.removeprefix("place:"), name, f"{where}." if where else "")
        else:
            out[key] = target
    return out


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
                aliases=aliases_of(export["name"], export["aliases"]), defaults=export["defaults"],
                kind_fields={k: tuple(v) for k, v in export["kind_fields"].items()},
                kind_neutral={k: tuple(v) for k, v in export["kind_neutral"].items()},
                kind_values={k: {v: tuple(kinds) for v, kinds in values.items()}
                             for k, values in export["kind_values"].items()},
                kind_descriptions=export["kind_descriptions"])


@dataclass
class RustResult(Result):
    """A Rust call's reply as a :class:`Result`: :meth:`to_json` is the envelope Rust
    wrote, key for key (``try_call`` included, which today's errors do not carry), so
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
                                        details=error.get("details"))
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

    def update(self, doc: str, parts: Mapping[str, bytes | None], *, context: Any = None) -> int:
        """Hand over a version's touched parts (``None`` removes one): the new version."""
        version, _changed, _attempts = self.session.update(doc, dict(parts), context=context)
        return version

    def dispatch(self, name: str, arguments: Mapping[str, Any] | str, *,
                 context: Any = None) -> RustResult:
        """Run the tool ``name``; its reply as a :class:`RustResult`."""
        text = arguments if isinstance(arguments, str) else json.dumps(arguments)
        reply, _images, _app = self.session.call(name, text, context=context)
        return RustResult.of(reply, name)

    def definitions(self, provider: str, **options: Any) -> list[dict[str, Any]]:
        """The tools' definitions for ``provider`` (``anthropic``, ``openai-responses``,
        ``openai-chat``), by today's adapters."""
        return definitions_for(provider, self.tools, **options)

    @staticmethod
    def results(provider: str, results: Iterable[tuple[str, Result]], **options: Any) -> Any:
        """What answers one assistant turn for ``provider``, by today's adapters."""
        return results_for(provider, list(results), **options)

#!/usr/bin/env python3
"""The differential jobs of a gates run: how many shards each mode gets and which modes share
a job (tools/gates.py ``matrix``, gates.yml's plan job).

A job waits for a runner far longer than most modes take to run (GitHub gives a repository
about twenty runners at once), so a run asks for as few jobs as its work allows:

* **cold** (the store of docx-agent's results for the reference code the run checks out --
  ``gates.py cache-key``, restored by the plan job -- does not cover every document the mode
  runs, by the store's coverage: tools/gates_store.py; or there is no store): the mode in
  COLD_SHARDS shards, as before the reference cache was warm;
* **warm** (the store covers the mode): the mode in as many shards as its recorded warm time needs at
  SHARD_TARGET seconds a shard (one, for all but the slowest), and the modes whose warm time
  is under BUNDLE_UNDER seconds run one after another in one job per gate (a *bundle*, at
  most BUNDLE_LIMIT seconds of work) -- one checkout and setup for all of them;
* the reads gate does not use the cache: always as warm;
* an explicit ``--shards N`` wins: every mode in N shards, none bundled.

The warm times are DURATIONS (tools/gates_durations.json): per gate and mode, a whole warm
mode's differential time in seconds and the documents it ran.  ``gates.py durations DIR``
writes them from a run's downloaded ``diff-*`` logs (the summary job prints them too); a
mode with no recorded time runs in a job of its own.  On the fixtures and reads gates a
mode's time is scaled to the fixtures the run picks (``--fixtures auto`` runs a few).

Each job is ``{"gate", "modes", "shard", "label"}``: ``modes`` a comma list (one mode, or a
bundle's), ``label`` the modes joined with ``+`` (the job's logs are ``diff-<gate>-<label>-<k>``,
with each mode's results in its own folder ``<gate>-<mode>-<k>``).
"""

from __future__ import annotations

import json
import math
from collections.abc import Collection
from pathlib import Path

DURATIONS = Path(__file__).resolve().parent / "gates_durations.json"

#: Shards per edit mode when the reference cache holds nothing for the run's reference code.
COLD_SHARDS = 3
#: A warm shard's work (s): a mode slower than this warm gets more shards.
SHARD_TARGET = 1200
#: A mode whose warm time (s) is under this shares a job with the gate's other small modes.
BUNDLE_UNDER = 120
#: A bundle's work at most (s); more small modes start another bundle.
BUNDLE_LIMIT = 900
#: The share of a mode's cases docx-agent may compute in a run whose time counts as warm.
WARM_COMPUTED = 0.05
#: The gates that do not use the reference cache: warm whatever the lookup found.
NO_CACHE = ("reads",)


def load_durations(path: Path = DURATIONS) -> dict[str, dict[str, dict]]:
    return json.loads(path.read_text()) if path.exists() else {}


def estimate(durations: dict, gate: str, mode: str, documents: int | None = None) -> float | None:
    """A mode's warm time (s) on ``documents`` documents (scaled from the recorded run's), or
    the recorded time when either count is unknown; None when nothing is recorded."""
    found = durations.get(gate, {}).get(mode)
    if not found:
        return None
    seconds, recorded = float(found["seconds"]), found.get("documents")
    if documents is not None and recorded:
        return seconds * documents / recorded
    return seconds


def _job(gate: str, modes: list[str], shard: str) -> dict:
    return {"gate": gate, "modes": ",".join(modes), "shard": shard, "label": "+".join(modes)}


def gate_jobs(gate: str, modes: list[str], *, warm: Collection[str], shards: int | None,
              estimates: dict[str, float | None]) -> list[dict]:
    """The jobs that run ``modes`` of ``gate`` (see the module's docstring); ``warm``: the
    modes the reference cache covers; ``estimates``: each mode's warm time (s) or None."""
    if shards:
        return [_job(gate, [mode], f"{k}/{shards}") for mode in modes for k in range(1, shards + 1)]
    cold = [] if gate in NO_CACHE else [mode for mode in modes if mode not in warm]
    # the cold modes' jobs first: the longest, they start before the pool fills
    first = [_job(gate, [mode], f"{k}/{COLD_SHARDS}") for mode in cold for k in range(1, COLD_SHARDS + 1)]
    jobs, bundles = [], []
    for mode in modes:
        if mode in cold:
            continue
        seconds = estimates.get(mode)
        if seconds is not None and seconds < BUNDLE_UNDER:
            # first fit, in the modes' order: a mode joins the first bundle with room for it
            room = next((b for b in bundles if b[1] + seconds <= BUNDLE_LIMIT), None)
            if room is None:
                room = [[], 0.0]
                bundles.append(room)
            room[0].append(mode)
            room[1] += seconds
            continue
        n = 1 if seconds is None else max(1, math.ceil(seconds / SHARD_TARGET))
        jobs += [_job(gate, [mode], f"{k}/{n}") for k in range(1, n + 1)]
    return first + [_job(gate, members, "1/1") for members, _ in bundles] + jobs


def warm_durations(summaries: list[dict], defaults: dict[str, int]) -> dict[str, dict[str, dict]]:
    """Each (gate, mode)'s warm time from one run's shard summaries (gates.py ``diff``'s
    summary.json): the shards' differential times added up, for a mode whose every shard ran
    without crashing, warm (docx-agent computed at most WARM_COMPUTED of its cases) and
    unsampled (every 1, ``defaults``' per-tool for its gate)."""
    groups: dict[tuple[str, str], list[dict]] = {}
    for s in summaries:
        groups.setdefault((s["gate"], s["mode"]), []).append(s)
    out: dict[str, dict[str, dict]] = {}
    for (gate, mode), shards in sorted(groups.items()):
        total = int(shards[0]["shard"].split("/")[1])
        if len({s["shard"] for s in shards}) != total or any(s["crashed"] for s in shards):
            continue
        if any(s.get("every", 1) != 1 or s.get("per_tool", defaults.get(gate, 0)) != defaults.get(gate, 0)
               for s in shards):
            continue
        cases = sum(s["cases"] for s in shards)
        computed = sum(s.get("computed", 0) for s in shards)
        if gate not in NO_CACHE and computed > WARM_COMPUTED * max(cases, 1):
            continue
        out.setdefault(gate, {})[mode] = {"seconds": round(sum(s["wall"] for s in shards), 1),
                                          "documents": sum(s["documents"] for s in shards)}
    return out


def merged(old: dict[str, dict[str, dict]], new: dict[str, dict[str, dict]]) -> dict[str, dict[str, dict]]:
    """``old`` with ``new``'s times in place of its own (sorted, as the file keeps them)."""
    out = {gate: dict(modes) for gate, modes in old.items()}
    for gate, modes in new.items():
        out.setdefault(gate, {}).update(modes)
    return {gate: dict(sorted(modes.items())) for gate, modes in sorted(out.items())}


def write_durations(new: dict[str, dict[str, dict]], path: Path = DURATIONS) -> None:
    path.write_text(json.dumps(merged(load_durations(path), new), indent=1) + "\n")

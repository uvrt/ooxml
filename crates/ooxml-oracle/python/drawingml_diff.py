#!/usr/bin/env python3
"""The DrawingML differential harness (test support; phase P3, lane P3a).

Reads each file as ooxml-common reads DrawingML -- every XML part of a package (or a bare
XML file, such as ooxml-common's golden fixtures), the theme's colour and format schemes,
every shape tree, ``spPr``, text body, shape style and colour choice -- and resolves what
it read the way ooxml-common's consumers compose its resolution (``resolve_color``,
``apply_transforms``, a style reference's colour standing for ``phClr``), under
PowerPoint's rules and Word's, with the part's theme and colour maps. It writes that in the
JSON form the Rust ``drawingml-model`` binary writes (``crates/ooxml-oracle/src/dml/``),
runs the binary on the same files, and compares the two field by field. Every difference
is printed with its JSON path; the exit status is 1 when there is any.

    python crates/ooxml-oracle/python/drawingml_diff.py --rust target/release/drawingml-model a.pptx b.docx
    python crates/ooxml-oracle/python/drawingml_diff.py --rust ... --tree DIR    # every .pptx/.docx/.xlsx below DIR
    python crates/ooxml-oracle/python/drawingml_diff.py --rust ... --presets     # the preset table, guides evaluated
    python crates/ooxml-oracle/python/drawingml_diff.py --rust ... --swatches N  # N generated colour swatches

The rules the two sides share: parts in name order; a part's theme is the first ``theme``
relationship of it or of the layout or master it follows, else the package's first theme
part; its colour maps are its master's, its layout's and its own (``p:clrMap`` or
``p:clrMapOvr``). Python's behaviour is not changed: this only reads ooxml-common's (and
pptx2svg's ``parse_color_map``) public functions.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import math
import posixpath
import random
import subprocess
import sys
import zipfile
from pathlib import Path
from types import SimpleNamespace
from xml.etree import ElementTree as ET

ROOT = Path(__file__).resolve().parents[3]
for package in ("ooxml-common", "pptx2svg"):
    source = ROOT / "python" / package / "src"
    if str(source) not in sys.path:
        sys.path.insert(0, str(source))

from ooxml_common.drawingml import color as dml_color  # noqa: E402
from ooxml_common.drawingml import model as m  # noqa: E402
from ooxml_common.drawingml import source as s  # noqa: E402
from ooxml_common.drawingml.guides import evaluate_guides  # noqa: E402
from ooxml_common.drawingml.preset_text_rects import PRESET_TEXT_RECTS  # noqa: E402
from ooxml_common.drawingml.presets import PRESETS  # noqa: E402
from ooxml_common.drawingml.read import (  # noqa: E402
    parse_color_node, parse_color_scheme, parse_effect_list, parse_fill, parse_format_scheme, parse_geometry,
    parse_geometry_spec, parse_outline, parse_shape_style, parse_text_rect, parse_transform,
)
from ooxml_common.drawingml.read_text import parse_text_body  # noqa: E402
from ooxml_common.drawingml.read_tree import parse_shape_tree  # noqa: E402
from ooxml_common.xmlutil import child, local_name  # noqa: E402
from pptx2svg.parse.parts import parse_color_map, parse_color_map_override  # noqa: E402

COLOUR_ELEMENTS = ("srgbClr", "schemeClr", "sysClr", "prstClr", "scrgbClr", "hslClr")
RULES = (("powerpoint", dml_color.POWERPOINT), ("word", dml_color.WORD))
ERROR = {"error": "colour"}


# -- the source model as JSON ----------------------------------------------------------------


def dump(value):
    """A dataclass as ``{"@": class, field: value...}`` (every field), a dict as
    ``{"{}": [[key, value]...]}``, tuples and lists as lists, a non-finite float as
    ``{"float": repr}``."""
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        out = {"@": type(value).__name__}
        for item in dataclasses.fields(value):
            out[item.name] = dump(getattr(value, item.name))
        return out
    if isinstance(value, dict):
        return {"{}": [[dump(k), dump(v)] for k, v in value.items()]}
    if isinstance(value, (list, tuple)):
        return [dump(v) for v in value]
    if isinstance(value, float) and not math.isfinite(value):
        return {"float": repr(value)}
    return value


def num(value):
    return dump(float(value)) if isinstance(value, (int, float)) and not isinstance(value, bool) else value


# -- resolution, as the consumers compose ooxml-common's ----------------------------------------


def colour_json(c):
    return None if c is None else {"hex": c.hex, "alpha": num(c.alpha)}


def reference_colour(ctx, colour, reference):
    if colour is None or colour.kind != "scheme" or colour.scheme != "phClr":
        return dml_color.resolve_color(ctx, colour)
    if reference is None:
        return None
    resolved = dml_color.apply_transforms(reference.hex, colour.transforms, ctx.rules)
    if reference.alpha < 1 and all(t.kind != "alpha" for t in colour.transforms):
        resolved = m.ResolvedColor(hex=resolved.hex, alpha=reference.alpha)
    return resolved


def fill_json(ctx, fill, reference):
    if isinstance(fill, s.SourceNoFill):
        return {"@": "NoFill"}
    if isinstance(fill, s.SourceSolidFill):
        c = reference_colour(ctx, fill.color, reference)
        return None if c is None else {"@": "SolidFill", "color": colour_json(c)}
    if isinstance(fill, s.SourceGradientFill):
        stops = []
        for stop in fill.stops:
            c = reference_colour(ctx, stop.color, reference)
            if c is not None:
                stops.append({"position": num(stop.position), "color": colour_json(c)})
        if not stops:
            return None
        return {"@": "GradientFill", "stops": stops, "angle": num(fill.angle / 60000),
                "gradient_type": fill.gradient_type, "center_x": dump(fill.center_x), "center_y": dump(fill.center_y),
                "path": fill.path, "focus": dump(fill.focus), "scaled": fill.scaled,
                "rotate_with_shape": fill.rotate_with_shape}
    if isinstance(fill, s.SourcePatternFill):
        fg = reference_colour(ctx, fill.foreground_color, reference)
        bg = reference_colour(ctx, fill.background_color, reference)
        if fg is None or bg is None:
            return None
        return {"@": "PatternFill", "preset": fill.preset, "foreground_color": colour_json(fg),
                "background_color": colour_json(bg)}
    if isinstance(fill, s.SourceImageFill):
        return {"@": "ImageFill", "blip": fill.blip_relationship_id}
    return None


def outline_json(ctx, outline, reference):
    paint = fill_json(ctx, outline.fill, reference) if outline.fill is not None else None
    if paint is not None and paint["@"] == "NoFill":
        return None
    end = lambda e: None if e is None else {"type": e.type, "width": e.width, "length": e.length}  # noqa: E731
    return {"@": "Outline", "width": num(outline.width if outline.width is not None else 9525),
            "fill": paint if paint is not None and paint["@"] in ("SolidFill", "GradientFill") else None,
            "dash_style": outline.dash_style or "solid",
            "custom_dash": dump(list(outline.custom_dash)) if outline.custom_dash else None,
            "line_cap": outline.line_cap, "line_join": outline.line_join, "head_end": end(outline.head_end),
            "tail_end": end(outline.tail_end), "compound": outline.compound}


def effects_json(ctx, effects, reference):
    out = {"@": "EffectList", "outer_shadow": None, "inner_shadow": None, "glow": None, "soft_edge": None}
    shadow = effects.outer_shadow
    if shadow is not None and (c := reference_colour(ctx, shadow.color, reference)) is not None:
        out["outer_shadow"] = {"blur_radius": num(shadow.blur_radius), "distance": num(shadow.distance),
                               "direction": num(shadow.direction / 60000), "color": colour_json(c),
                               "alignment": shadow.alignment, "rotate_with_shape": shadow.rotate_with_shape}
    inner = effects.inner_shadow
    if inner is not None and (c := reference_colour(ctx, inner.color, reference)) is not None:
        out["inner_shadow"] = {"blur_radius": num(inner.blur_radius), "distance": num(inner.distance),
                               "direction": num(inner.direction / 60000), "color": colour_json(c)}
    if effects.glow is not None and (c := reference_colour(ctx, effects.glow.color, reference)) is not None:
        out["glow"] = {"radius": num(effects.glow.radius), "color": colour_json(c)}
    if effects.soft_edge is not None:
        out["soft_edge"] = {"radius": num(effects.soft_edge.radius)}
    return None if all(out[k] is None for k in ("outer_shadow", "inner_shadow", "glow", "soft_edge")) else out


def caught(function, *args):
    try:
        return function(*args)
    except ValueError:
        return ERROR


def style_refs(ctx, style, formats):
    def reference(ref):
        return dml_color.resolve_color(ctx, ref.color)

    out = {"fill_ref": None, "line_ref": None, "effect_ref": None}
    ref = style.fill_ref
    if ref is not None and ref.idx != 0:
        styles, index = ((formats.bg_fill_styles, ref.idx - 1001) if ref.idx >= 1000
                         else (formats.fill_styles, ref.idx - 1))
        if 0 <= index < len(styles):
            out["fill_ref"] = caught(lambda: fill_json(ctx, styles[index], reference(ref)))
    ref = style.line_ref
    if ref is not None and ref.idx != 0 and 1 <= ref.idx <= len(formats.line_styles):
        out["line_ref"] = caught(lambda: outline_json(ctx, formats.line_styles[ref.idx - 1], reference(ref)))
    ref = style.effect_ref
    if ref is not None and 1 <= ref.idx <= len(formats.effect_styles) and formats.effect_styles[ref.idx - 1]:
        out["effect_ref"] = caught(lambda: effects_json(ctx, formats.effect_styles[ref.idx - 1], reference(ref)))
    return out


# -- a package's parts ---------------------------------------------------------------------------


class Source:
    def __init__(self, path: Path):
        self.zip = zipfile.ZipFile(path) if path.suffix != ".xml" else None
        self.names = sorted(n for n in self.zip.namelist() if not n.endswith("/")) if self.zip else []
        self.trees: dict[str, ET.Element | None] = {}

    def tree(self, name):
        if name not in self.trees:
            try:
                self.trees[name] = ET.fromstring(self.zip.read(name))
            except (KeyError, ET.ParseError):
                self.trees[name] = None
        return self.trees[name]

    def related(self, part, suffixes):
        rels_name = posixpath.join(posixpath.dirname(part), "_rels", posixpath.basename(part) + ".rels")
        if rels_name not in self.names:
            return None
        root = self.tree(rels_name)
        if root is None:
            return None
        for rel in root:
            if rel.tag != "{http://schemas.openxmlformats.org/package/2006/relationships}Relationship":
                continue
            kind, target, mode = rel.get("Type"), rel.get("Target"), rel.get("TargetMode")
            if kind is None or target is None or rel.get("Id") is None or mode == "External":
                continue
            if not any(kind.endswith(suffix) for suffix in suffixes):
                continue
            resolved = posixpath.normpath(posixpath.join(posixpath.dirname(part), target)).lstrip("/")
            if target.startswith("/"):
                resolved = posixpath.normpath(target).lstrip("/")
            if resolved in self.names:
                return resolved
        return None

    def parent(self, part):
        return self.related(part, ("/slideLayout", "/slideMaster"))

    def theme_part(self, part, depth=0):
        found = self.related(part, ("/theme",))
        if found:
            return found
        parent = self.parent(part) if depth < 4 else None
        if parent:
            return self.theme_part(parent, depth + 1)
        for name in self.names:
            root = self.tree(name) if name.endswith(".xml") else None
            if root is not None and local_name(root.tag) == "theme":
                return name
        return None

    def map_layers(self, part, depth=0):
        parent = self.parent(part) if depth < 4 else None
        layers = self.map_layers(parent, depth + 1) if parent else []
        root = self.tree(part)
        if root is not None:
            own = parse_color_map(child(root, "clrMap")) or parse_color_map_override(child(root, "clrMapOvr"))
            layers.append(own)
        return layers


def theme_of(root):
    elements = child(root, "themeElements")
    return SimpleNamespace(color_scheme=parse_color_scheme(child(elements, "clrScheme")),
                           format_scheme=parse_format_scheme(child(elements, "fmtScheme")))


def map_json(mapping):
    return [[k, v] for k, v in sorted(mapping.items())]


def contexts(source, name):
    theme_name = source.theme_part(name) if source.zip else None
    theme_root = source.tree(theme_name) if theme_name else None
    theme = theme_of(theme_root) if theme_root is not None else None
    layers = [layer.mapping if layer is not None else None
              for layer in (source.map_layers(name) if source.zip else [])]
    effective = dict(dml_color.DEFAULT_COLOR_MAP)
    for layer in layers:
        if layer:
            effective.update(layer)
    maps = {"layers": [map_json(l) if l is not None else None for l in layers], "effective": map_json(effective)}
    rules = [(label, dml_color.ColorContext(theme, dict(effective), rules=rules)) for label, rules in RULES]
    formats = theme.format_scheme if theme is not None else s.SourceFormatScheme()
    return rules, formats, maps


def sp_pr(e, rules):
    fill, line, effects = parse_fill(e), parse_outline(e), parse_effect_list(child(e, "effectLst"))
    model = {"fill": dump(fill), "outline": dump(line), "geometry": dump(parse_geometry(e)),
             "spec": dump(parse_geometry_spec(e)), "text_rect": dump(parse_text_rect(e)),
             "xfrm": dump(parse_transform(e)), "effects": dump(effects)}
    resolved = {}
    for label, ctx in rules:
        resolved[label] = {
            "fill": None if fill is None else caught(fill_json, ctx, fill, None),
            "outline": None if line is None else caught(outline_json, ctx, line, None),
            "effects": None if effects is None else caught(effects_json, ctx, effects, None),
        }
    return {"model": model, "resolved": resolved}


def part(source, name, root):
    rules, formats, maps = contexts(source, name)
    record = {}
    if local_name(root.tag) == "theme":
        theme = theme_of(root)
        record["theme"] = {"colours": [[k, dump(v)] for k, v in theme.color_scheme.items()],
                           "formats": dump(theme.format_scheme)}
    trees, sppr, bodies, styles, colours = [], [], [], [], []
    for e in root.iter():
        if not isinstance(e.tag, str):
            continue
        local = local_name(e.tag)
        if local == "spTree":
            trees.append(dump(parse_shape_tree(e)))
        elif local == "spPr":
            sppr.append(sp_pr(e, rules))
        elif local in ("txBody", "rich", "txPr"):
            bodies.append(dump(parse_text_body(e)))
        elif local == "style":
            style = parse_shape_style(e)
            styles.append({"model": dump(style), "resolved": {
                label: None if style is None else style_refs(ctx, style, formats) for label, ctx in rules}})
        elif local in COLOUR_ELEMENTS:
            colour = parse_color_node(e)
            resolved = None if colour is None else {
                label: caught(lambda: colour_json(dml_color.resolve_color(ctx, colour))) for label, ctx in rules}
            colours.append({"source": dump(colour), "resolved": resolved})
    record.update({"maps": maps, "trees": trees, "sppr": sppr, "bodies": bodies, "styles": styles,
                   "colours": colours})
    return record


def python_record(path: Path) -> dict:
    source = Source(path)
    if source.zip is None:
        return {"file": str(path), "parts": {str(path): part(source, str(path), ET.parse(path).getroot())}}
    parts = {}
    for name in source.names:
        if not name.endswith(".xml") or name == "[Content_Types].xml":
            continue
        root = source.tree(name)
        parts[name] = {"error": "parse"} if root is None else part(source, name, root)
    return {"file": str(path), "parts": parts}


# -- the preset table and colour swatches ----------------------------------------------------------


def presets_record() -> list:
    rows = []
    for name, (adjustments, guides, paths) in PRESETS.items():
        adjust = [(n, f"val {d}") for n, d in adjustments]

        def evaluate(precise, adjust=adjust, guides=guides):
            values = evaluate_guides([adjust, list(guides)], 120, 72, precise=precise)
            return {k: num(v) for k, v in sorted(values.items())}

        rows.append({"name": name, "adjustments": [[n, num(d)] for n, d in adjustments],
                     "guides": [list(g) for g in guides],
                     "paths": [[f, st, None if sp is None else [num(sp[0]), num(sp[1])], [list(c) for c in cmds]]
                               for f, st, sp, cmds in paths],
                     "text_rect": list(PRESET_TEXT_RECTS[name]), "precise": evaluate(True), "rounded": evaluate(False)})
    return rows


KINDS = ("lumMod", "lumOff", "tint", "shade", "alpha", "satMod", "satOff", "hueMod", "hueOff", "gray", "inv",
         "comp", "gamma", "invGamma", "alphaMod", "alphaOff")
VALUES = (0, 10000, 25000, 33333, 50000, 66667, 75000, 95000, 100000, 120000, 150000, 200000, -25000, -50000,
          5400000, 21600000)


def swatch_cases(count: int) -> list[dict]:
    rng = random.Random(20261117)
    bases = ["000000", "FFFFFF", "808080", "4472C4", "ED7D31", "70AD47", "FFE699", "7F7F7F", "1F1F1F", "C00000"]
    cases = []
    for k in range(count):
        base = bases[k % len(bases)] if k < 2 * len(bases) else f"{rng.randrange(1 << 24):06X}"
        transforms = [[rng.choice(KINDS), rng.choice(VALUES) if rng.random() < 0.6 else rng.randrange(-60000, 260000)]
                      for _ in range(rng.randrange(0, 5))]
        linear = [rng.random() for _ in range(3)] if k % 7 == 3 else None
        cases.append({"hex": base, "linear": linear, "transforms": transforms})
    return cases


def swatch_answer(case: dict) -> dict:
    transforms = [m.ColorTransform(kind=k, value=v) for k, v in case["transforms"]]
    out = {}
    for label, rules in RULES:
        try:
            if case["linear"] is not None:
                c = dml_color.apply_transforms_linear(tuple(case["linear"]), transforms, rules)
            else:
                c = dml_color.apply_transforms(case["hex"], transforms, rules)
            out[label] = colour_json(c)
        except ValueError:
            out[label] = ERROR
    return out


# -- comparing ----------------------------------------------------------------------------------------


def same_number(a, b) -> bool:
    if isinstance(a, bool) or isinstance(b, bool):
        return a is b
    return float(a) == float(b)


def diff(a, b, path: str, out: list) -> None:
    if isinstance(a, (int, float)) and isinstance(b, (int, float)) and not (isinstance(a, bool) or isinstance(b, bool)):
        if not same_number(a, b):
            out.append(f"{path}: python {a!r} rust {b!r}")
        return
    if type(a) is not type(b):
        out.append(f"{path}: python {json.dumps(a)[:200]} rust {json.dumps(b)[:200]}")
        return
    if isinstance(a, dict):
        for key in sorted(set(a) | set(b)):
            if key not in a or key not in b:
                out.append(f"{path}.{key}: only in {'python' if key in a else 'rust'}")
            else:
                diff(a[key], b[key], f"{path}.{key}", out)
    elif isinstance(a, list):
        if len(a) != len(b):
            out.append(f"{path}: python {len(a)} item(s), rust {len(b)}")
        for i, (x, y) in enumerate(zip(a, b)):
            diff(x, y, f"{path}[{i}]", out)
    elif a != b:
        out.append(f"{path}: python {a!r} rust {b!r}")


def run_rust(binary: str, args: list[str], stdin: str | None = None) -> list:
    done = subprocess.run([binary, *args], input=stdin, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise SystemExit(f"{binary} failed: {done.stderr.strip()}")
    return [json.loads(line) for line in done.stdout.splitlines() if line.strip()]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--rust", required=True, help="the drawingml-model binary")
    parser.add_argument("--tree", action="append", default=[], help="every .pptx/.docx/.xlsx below this folder")
    parser.add_argument("--presets", action="store_true")
    parser.add_argument("--swatches", type=int, default=0)
    parser.add_argument("--json", help="write the per-file counts here")
    parser.add_argument("files", nargs="*")
    args = parser.parse_args()
    total, summary = 0, {}
    if args.presets:
        out: list[str] = []
        diff(presets_record(), run_rust(args.rust, ["--presets"])[0], "presets", out)
        summary["presets"] = len(out)
        total += len(out)
        print(*out[:50], sep="\n") if out else None
        print(f"presets: {len(PRESETS)} preset(s), {len(out)} difference(s)")
    if args.swatches:
        cases = swatch_cases(args.swatches)
        rust = run_rust(args.rust, ["--swatches"], "\n".join(json.dumps(c) for c in cases))
        out = []
        for i, (case, answer) in enumerate(zip(cases, rust)):
            diff(swatch_answer(case), answer, f"swatch[{i}] {json.dumps(case)}", out)
        summary["swatches"] = len(out)
        total += len(out)
        print(*out[:50], sep="\n") if out else None
        print(f"swatches: {len(cases)} case(s), {len(out)} difference(s)")
    files = [Path(f) for f in args.files]
    for folder in args.tree:
        files += sorted(p for p in Path(folder).rglob("*") if p.suffix in (".pptx", ".docx", ".xlsx", ".pptm", ".docm"))
    for path in files:
        out = []
        rust = run_rust(args.rust, [str(path)])[0]
        diff(python_record(path), rust, "", out)
        summary[str(path)] = len(out)
        total += len(out)
        for line in out[:40]:
            print(f"  {line}")
        print(f"{path}: {len(out)} difference(s)")
    print(f"{len(files)} file(s), {total} difference(s)")
    if args.json:
        Path(args.json).write_text(json.dumps(summary, indent=1) + "\n", encoding="utf-8")
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main())

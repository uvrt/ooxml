"""The Rust layout's measuring input, made from docx2svg's model of a document (lane S's
exporter, moved here from ooxml-core's test harness): each paragraph's runs as text and
character formats, its label and geometry; the faces' advances and metrics the measuring
needs.  Every length is a :class:`Px` (device px) or :class:`Wd` (layout-unit widths), made
an integer over one denominator each when encoded."""
from __future__ import annotations

from fractions import Fraction

from docx2svg import linebreak
from docx2svg import lines as line_model
from docx2svg.resolve import character_format, resolve_mark, resolve_paragraph, resolve_run, script_half_points

class Unsupported(Exception):
    """What the Rust layout does not model (yet): the document is laid out by docx2svg, as
    before, never approximated."""


class Px:
    __slots__ = ("v",)

    def __init__(self, v):
        self.v = Fraction(v)


class Wd:
    __slots__ = ("v",)

    def __init__(self, v):
        self.v = Fraction(v)


RULES = {"auto": 0, "exact": 1, "atLeast": 2}
KINDS = {linebreak.GLYPH: 0, linebreak.SPACE: 1, linebreak.TAB: 2, linebreak.BREAK: 3, linebreak.SOFT_HYPHEN: 4,
         linebreak.OBJECT: 5}


class Exporter:
    def __init__(self, document, advances, metrics):
        self.document = document
        self.advances = advances
        self.metrics = metrics
        self.fonts: dict = {}  # (face, bold, italic) -> index
        self.font_list: list = []
        self.formats: dict = {}
        self.format_list: list = []
        self.styles: dict = {None: -1}
        self.chars: set = set()
        #: Lane F: ``(face, character)`` drawn (a face's glyph fallbacks are of these alone).
        self.drawn: set = set()
        self.kern_fonts: set = set()

    def style(self, name):
        if name not in self.styles:
            self.styles[name] = len(self.styles) - 1
        return self.styles[name]

    def font(self, face, bold, italic):
        key = (face, bool(bold), bool(italic))
        if key not in self.fonts:
            self.fonts[key] = len(self.font_list)
            self.font_list.append(key)
        return self.fonts[key]

    #: Lane F: the faces formats name, in the order first named (the mirror reads it per block).
    face_log: list | None = None

    def fmt(self, f, width_hp=None):
        """A character format's index: font, size, width size (a script's), position, border."""
        if self.face_log is not None and f.face and f.face not in self.face_log:
            self.face_log.append(f.face)
        font = self.font(f.face, f.bold, f.italic) if f.face else -1
        key = (font, f.half_points, width_hp if width_hp is not None else f.half_points, f.position or 0,
               tuple(f.border) if f.border else None)
        if key not in self.formats:
            self.formats[key] = len(self.format_list)
            self.format_list.append(key)
        return self.formats[key]

    def width_hp(self, f):
        if not f.vertical_align:
            return f.half_points
        m = self.metrics(f.face, f.bold, f.italic)
        return script_half_points(f.half_points, f.vertical_align, m)

    # -- a paragraph's measuring input -------------------------------------------------

    def measure_input(self, paragraph, section, label, pp=None):
        document = self.document
        pp = pp or resolve_paragraph(document, paragraph)
        runs = []
        for run in paragraph.runs:
            resolved = resolve_run(document, paragraph, run)
            if run.note_number:
                raise Unsupported("a note number: not exported")
            entry = {"text": run.text, "vanish": bool(resolved.get("vanish"))}
            if not entry["vanish"]:
                fmts = []
                # One character's format and what it adds, per distinct character of the
                # run (0.1.116): the same for each of its occurrences.
                seen: dict = {}
                caps = resolved.get("caps")
                for char in run.text:
                    index = seen.get(char)
                    if index is None:
                        f = character_format(resolved, char, document)
                        index = seen[char] = self.fmt(f, self.width_hp(f))
                        drawn = char.upper() if caps else char
                        self.chars.add(drawn)
                        self.chars.add(char)
                        self.drawn.add((f.face, drawn))
                    fmts.append(index)
                entry["fmts"] = fmts if len(set(fmts)) > 1 else (fmts[:1] or [self.fmt(character_format(resolved, " ", document))])
                entry["tracking"] = linebreak.twips_to_units(resolved.get("spacing") or 0) if resolved.get("spacing") else 0
                entry["scale"] = resolved.get("w") or 100
                entry["kern"] = resolved.get("kern") or 0
                entry["caps"] = bool(resolved.get("caps"))
                # a run's hyphen ("-") and space, for marks
                hy = character_format(resolved, "-", document)
                entry["hyphen_fmt"] = self.fmt(hy, self.width_hp(hy))
                self.chars.add("-")
                if entry["kern"]:
                    for k in set(entry["fmts"]):
                        self.kern_fonts.add(self.format_list[k][0])
            entry["marks"] = [[pos, kind] for pos, kind in run.breaks]
            for _, kind in run.breaks:
                if kind == "noBreakHyphen":
                    self.chars.add("\u2011")
                if kind not in ("tab", "br", "textWrapping", "page", "column", "cr", "noBreakHyphen", "softHyphen"):
                    raise Unsupported(f"mark {kind!r}: not exported")
            runs.append(entry)
        out = {"runs": runs}
        mark = resolve_mark(document, paragraph)
        out["mark_fmt"] = self.fmt(character_format(mark, " ", document))
        if label is not None:
            out["label"] = self.label_input(paragraph, label, pp, mark)
        # height items of the label (lines.label_format: the level's text with "1"s)
        lf = line_model.label_format(document, paragraph, pp, mark)
        if any(isinstance(x, line_model.PictureLabel) for x in lf):
            raise Unsupported("picture bullet: not exported")
        out["label_height_fmts"] = [self.fmt(x) for x in lf]
        g = linebreak.geometry(document, paragraph, section, pp)
        if g.hyphen_zone is not None or g.hyphen_gap is not None or g.hyphen_limit:
            raise Unsupported("hyphenation: not exported")
        out["geometry"] = {"first_start": Wd(g.first_start), "start": Wd(g.start), "right": Wd(g.right),
                           "tabs": [[Wd(t.position), t.alignment] for t in g.tabs],
                           "default_tab": Wd(g.default_tab),
                           "hanging_stop": Wd(g.hanging_stop) if g.hanging_stop is not None else None,
                           "squeeze": g.squeeze, "tab_origin": Wd(g.tab_origin)}
        out["rule"] = RULES[pp.get("spacing.lineRule", "auto")]
        out["line"] = pp.get("spacing.line", 240)
        out["columns"] = linebreak.is_multi_column(section)
        return out

    def label_input(self, paragraph, text, pp, mark):
        from docx2svg.resolve.cascade import Resolved

        document = self.document
        num_id, ilvl = pp.get("numPr.numId"), pp.get("numPr.ilvl", 0) or 0
        level = document.numbering.get(num_id, {}).get(ilvl) if num_id else None
        if level is None:
            return None
        if level.picture_bullet is not None:
            raise Unsupported("picture bullet: not exported")
        values = dict(mark.values)
        values.update(level.run)
        resolved = Resolved(values, dict(mark.origins))
        caps = bool(resolved.get("caps"))
        chars = []
        for char in text:
            f = character_format(resolved, char, document)
            drawn = char.upper() if caps else char
            self.chars.add(drawn)
            self.drawn.add((f.face, drawn))
            chars.append([drawn, self.fmt(f)])
        sp = character_format(resolved, " ", document)
        self.chars.add(" ")
        alignment = getattr(level, "alignment", None)
        return {"chars": chars, "align": "center" if alignment == "center" else (
            "right" if alignment in ("right", "end") else "left"),
                "suffix": level.suffix or "tab", "space_fmt": self.fmt(sp)}

    # -- docx2svg's own results ----------------------------------------------------------

    @staticmethod
    def height(h):
        return [Px(h.pitch), Px(h.text_above), Px(h.text_below), RULES[h.rule], h.line, Px(h.natural)]

    @staticmethod
    def ref(pieces, broken):
        return {"widths": [Wd(p.width) for p in pieces], "kinds": [KINDS[p.kind] for p in pieces],
                "lines": [[ln.start, ln.end, int(ln.forced) | 2 * int(ln.emergency) | 4 * int(ln.hyphenated)]
                          for ln in broken]}


def fonts_out(ex):
    advances, metrics = ex.advances, ex.metrics
    chars = sorted(ex.chars | {" ", "-"})
    font_out = []
    for k, (face, bold, italic) in enumerate(ex.font_list):
        m = metrics(face, bold, italic) if face else None
        adv = {}
        upm = None
        for c in chars:
            a = advances.advance(face, bold, italic, c) if face else None
            if a is not None:
                if upm is not None and upm != a[1]:
                    continue  # a glyph fallback of another unitsPerEm: left out (the kernel stops at it)
                adv[c] = a[0]
                upm = a[1]
        kern = []
        if k in ex.kern_fonts:
            for left in chars:
                for right in chars:
                    v = advances.kern(face, bold, italic, left, right)
                    if v:
                        kern.append([left, right, v])
        font_out.append({"face": face, "bold": bold, "italic": italic, "upm": upm or (m.units_per_em if m else 0),
                         "metrics": [m.units_per_em, m.ascent, m.descent, m.line_gap, m.superscript_size,
                                     m.subscript_size, m.superscript_offset, m.subscript_offset] if m else None,
                         "adv": adv, "kern": kern, "kern_known": k in ex.kern_fonts})
    return font_out

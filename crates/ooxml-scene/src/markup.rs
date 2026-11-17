//! Owns: the DrawingML painter's display list -- elements in SVG's own vocabulary
//! ([`Tag`]: groups, links, shapes, text with its spans, images, and the paint servers,
//! markers, clips and filters a drawing references), each attribute a name with a typed
//! value ([`Value`]: numbers, lists, points, transforms, path data, colours, references)
//! kept in the order its producer gave it -- and the writer that writes one as SVG text.
//!
//! The order is kept because the reference output (ooxml-common's string-built SVG) fixes
//! it, and a byte-for-byte equal writer must reproduce it; SVG gives it no meaning. Every
//! number goes through [`crate::drawing_num`] (or the form its value names), so nothing
//! written depends on how a producer formatted it.
//!
//! Must not: know any OOXML, parse SVG, or decide anything an element does not say.

use crate::drawnum::{drawing_num, factor, fixed2};
use crate::svg::escape;

/// An element's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tag {
    /// `g`.
    G,
    /// `a` (a hyperlink).
    A,
    /// `path`.
    Path,
    /// `rect`.
    Rect,
    /// `line`.
    Line,
    /// `ellipse`.
    Ellipse,
    /// `polygon`.
    Polygon,
    /// `text`.
    Text,
    /// `tspan`.
    Tspan,
    /// `image`.
    Image,
    /// `clipPath`.
    ClipPath,
    /// `linearGradient`.
    LinearGradient,
    /// `radialGradient`.
    RadialGradient,
    /// `stop`.
    Stop,
    /// `pattern`.
    Pattern,
    /// `marker`.
    Marker,
    /// `filter`.
    Filter,
    /// `feFlood`.
    FeFlood,
    /// `feComposite`.
    FeComposite,
    /// `feOffset`.
    FeOffset,
    /// `feGaussianBlur`.
    FeGaussianBlur,
    /// `feMerge`.
    FeMerge,
    /// `feMergeNode`.
    FeMergeNode,
    /// `feColorMatrix`.
    FeColorMatrix,
    /// `feComponentTransfer`.
    FeComponentTransfer,
    /// `feFuncR`.
    FeFuncR,
    /// `feFuncG`.
    FeFuncG,
    /// `feFuncB`.
    FeFuncB,
    /// `feFuncA`.
    FeFuncA,
    /// `feMorphology`.
    FeMorphology,
    /// `feBlend`.
    FeBlend,
    /// `feImage`.
    FeImage,
}

impl Tag {
    /// The name as written.
    pub fn name(self) -> &'static str {
        match self {
            Tag::G => "g",
            Tag::A => "a",
            Tag::Path => "path",
            Tag::Rect => "rect",
            Tag::Line => "line",
            Tag::Ellipse => "ellipse",
            Tag::Polygon => "polygon",
            Tag::Text => "text",
            Tag::Tspan => "tspan",
            Tag::Image => "image",
            Tag::ClipPath => "clipPath",
            Tag::LinearGradient => "linearGradient",
            Tag::RadialGradient => "radialGradient",
            Tag::Stop => "stop",
            Tag::Pattern => "pattern",
            Tag::Marker => "marker",
            Tag::Filter => "filter",
            Tag::FeFlood => "feFlood",
            Tag::FeComposite => "feComposite",
            Tag::FeOffset => "feOffset",
            Tag::FeGaussianBlur => "feGaussianBlur",
            Tag::FeMerge => "feMerge",
            Tag::FeMergeNode => "feMergeNode",
            Tag::FeColorMatrix => "feColorMatrix",
            Tag::FeComponentTransfer => "feComponentTransfer",
            Tag::FeFuncR => "feFuncR",
            Tag::FeFuncG => "feFuncG",
            Tag::FeFuncB => "feFuncB",
            Tag::FeFuncA => "feFuncA",
            Tag::FeMorphology => "feMorphology",
            Tag::FeBlend => "feBlend",
            Tag::FeImage => "feImage",
        }
    }
}

/// How the two or three numbers of a transform function are separated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sep {
    /// `, ` (`translate(1, 2)`).
    CommaSpace,
    /// `,` (`translate(1,2)`).
    Comma,
    /// ` ` (`rotate(30 1 2)`).
    Space,
}

impl Sep {
    fn text(self) -> &'static str {
        match self {
            Sep::CommaSpace => ", ",
            Sep::Comma => ",",
            Sep::Space => " ",
        }
    }
}

/// One function of a `transform` list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    /// `translate(x y)`.
    Translate(f64, f64, Sep),
    /// `scale(x y)`.
    Scale(f64, f64, Sep),
    /// `scale(x y)`, its factors written as multipliers (ten significant digits).
    ScaleFactor(f64, f64, Sep),
    /// `rotate(angle)`.
    Rotate(f64),
    /// `rotate(angle cx cy)`.
    RotateAbout(f64, f64, f64, Sep),
    /// `skewX(angle)`.
    SkewX(f64),
    /// `matrix(a b c d e f)`.
    Matrix([f64; 6]),
}

/// One command of path data: its letter and its operands in groups. Written as the letter,
/// then (when there are operands) a space and the groups -- each its numbers joined by
/// spaces -- joined by `, `.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    /// The command letter.
    pub letter: char,
    /// The operands, grouped.
    pub groups: Vec<Vec<f64>>,
}

impl Command {
    /// A command whose operands form one group.
    pub fn new(letter: char, operands: Vec<f64>) -> Command {
        let groups = if operands.is_empty() {
            Vec::new()
        } else {
            vec![operands]
        };
        Command { letter, groups }
    }
}

/// An attribute's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A number ([`drawing_num`]).
    Num(f64),
    /// A number with two fixed decimals.
    Fixed2(f64),
    /// A number followed by `%`.
    Percent(f64),
    /// Numbers joined by spaces.
    Nums(Vec<f64>),
    /// Points, each `x,y`, joined by spaces.
    Points(Vec<(f64, f64)>),
    /// A transform list, its functions joined by spaces.
    Transform(Vec<Op>),
    /// Path data, its commands joined by spaces.
    Path(Vec<Command>),
    /// A reference to a resource (`url(#id)`).
    Url(String),
    /// A keyword or colour, written as it stands.
    Word(String),
    /// Text, escaped for an attribute.
    Text(String),
}

/// An attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    /// Its name.
    pub name: &'static str,
    /// Its value.
    pub value: Value,
}

/// What an element holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Child {
    /// An element.
    Element(Element),
    /// Character data, escaped as text.
    Text(String),
}

/// An element: its name, its attributes in order, and its content -- `None` for an empty
/// element written `<name .../>`, `Some` (perhaps empty) for one with a closing tag.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Its name.
    pub tag: Tag,
    /// Its attributes, in order.
    pub attrs: Vec<Attr>,
    /// Its content.
    pub children: Option<Vec<Child>>,
}

impl Element {
    /// An empty element (`<name .../>`).
    pub fn empty(tag: Tag) -> Element {
        Element {
            tag,
            attrs: Vec::new(),
            children: None,
        }
    }

    /// An element with a closing tag and no content yet.
    pub fn open(tag: Tag) -> Element {
        Element {
            tag,
            attrs: Vec::new(),
            children: Some(Vec::new()),
        }
    }

    /// `self` with `name="value"` appended.
    #[must_use]
    pub fn with(mut self, name: &'static str, value: Value) -> Element {
        self.attrs.push(Attr { name, value });
        self
    }

    /// Append `name="value"`.
    pub fn set(&mut self, name: &'static str, value: Value) {
        self.attrs.push(Attr { name, value });
    }

    /// A number attribute appended.
    #[must_use]
    pub fn num(self, name: &'static str, value: f64) -> Element {
        self.with(name, Value::Num(value))
    }

    /// A keyword or colour attribute appended.
    #[must_use]
    pub fn word(self, name: &'static str, value: &str) -> Element {
        self.with(name, Value::Word(value.to_owned()))
    }

    /// Append a child element (making the element one with a closing tag).
    pub fn push(&mut self, child: Element) {
        self.children.get_or_insert_with(Vec::new).push(Child::Element(child));
    }

    /// Append character data.
    pub fn push_text(&mut self, text: &str) {
        self.children
            .get_or_insert_with(Vec::new)
            .push(Child::Text(text.to_owned()));
    }

    /// `self` holding `children`.
    #[must_use]
    pub fn holding(mut self, children: Vec<Element>) -> Element {
        let list = self.children.get_or_insert_with(Vec::new);
        list.extend(children.into_iter().map(Child::Element));
        self
    }

    /// Put `attrs` before the element's own attributes.
    pub fn prepend(&mut self, attrs: Vec<Attr>) {
        let rest = std::mem::take(&mut self.attrs);
        self.attrs = attrs;
        self.attrs.extend(rest);
    }

    /// The element as SVG text.
    pub fn to_svg(&self) -> String {
        let mut out = String::new();
        write_element(&mut out, self);
        out
    }
}

/// `element` as SVG, appended to `out`.
pub fn write_element(out: &mut String, element: &Element) {
    out.push('<');
    out.push_str(element.tag.name());
    for attr in &element.attrs {
        out.push(' ');
        out.push_str(attr.name);
        out.push_str("=\"");
        write_value(out, &attr.value);
        out.push('"');
    }
    match &element.children {
        None => out.push_str("/>"),
        Some(children) => {
            out.push('>');
            for child in children {
                match child {
                    Child::Element(e) => write_element(out, e),
                    Child::Text(t) => out.push_str(&escape(t, false)),
                }
            }
            out.push_str("</");
            out.push_str(element.tag.name());
            out.push('>');
        }
    }
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Num(v) => out.push_str(&drawing_num(*v)),
        Value::Fixed2(v) => out.push_str(&fixed2(*v)),
        Value::Percent(v) => {
            out.push_str(&drawing_num(*v));
            out.push('%');
        }
        Value::Nums(values) => out.push_str(&joined(values)),
        Value::Points(points) => {
            let written: Vec<String> = points
                .iter()
                .map(|(x, y)| format!("{},{}", drawing_num(*x), drawing_num(*y)))
                .collect();
            out.push_str(&written.join(" "));
        }
        Value::Transform(ops) => {
            let written: Vec<String> = ops.iter().map(op_text).collect();
            out.push_str(&written.join(" "));
        }
        Value::Path(commands) => out.push_str(&path_text(commands)),
        Value::Url(id) => {
            out.push_str("url(#");
            out.push_str(&escape(id, true));
            out.push(')');
        }
        Value::Word(word) => out.push_str(word),
        Value::Text(text) => out.push_str(&escape(text, true)),
    }
}

/// Path data as written.
pub fn path_text(commands: &[Command]) -> String {
    let written: Vec<String> = commands
        .iter()
        .map(|c| {
            if c.groups.is_empty() {
                return c.letter.to_string();
            }
            let groups: Vec<String> = c.groups.iter().map(|g| joined(g)).collect();
            format!("{} {}", c.letter, groups.join(", "))
        })
        .collect();
    written.join(" ")
}

fn joined(values: &[f64]) -> String {
    values.iter().map(|v| drawing_num(*v)).collect::<Vec<_>>().join(" ")
}

fn op_text(op: &Op) -> String {
    let two =
        |name: &str, a: f64, b: f64, sep: Sep| format!("{name}({}{}{})", drawing_num(a), sep.text(), drawing_num(b));
    match *op {
        Op::Translate(x, y, sep) => two("translate", x, y, sep),
        Op::Scale(x, y, sep) => two("scale", x, y, sep),
        Op::ScaleFactor(x, y, sep) => format!("scale({}{}{})", factor(x), sep.text(), factor(y)),
        Op::Rotate(a) => format!("rotate({})", drawing_num(a)),
        Op::RotateAbout(a, x, y, sep) => format!(
            "rotate({}{s}{}{s}{})",
            drawing_num(a),
            drawing_num(x),
            drawing_num(y),
            s = sep.text()
        ),
        Op::SkewX(a) => format!("skewX({})", drawing_num(a)),
        Op::Matrix(m) => format!("matrix({})", joined(&m)),
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, Element, Op, Sep, Tag, Value};

    #[test]
    fn an_element_is_written_with_its_attributes_in_order() {
        let mut g = Element::open(Tag::G).with(
            "transform",
            Value::Transform(vec![Op::Translate(35.268, 283.52, Sep::CommaSpace)]),
        );
        g.push(
            Element::empty(Tag::Line)
                .num("stroke-width", 0.666_67)
                .word("stroke", "#000000")
                .word("fill", "none")
                .num("x1", 0.0),
        );
        assert_eq!(
            g.to_svg(),
            "<g transform=\"translate(35.268, 283.52)\"><line stroke-width=\"0.667\" stroke=\"#000000\" \
             fill=\"none\" x1=\"0\"/></g>"
        );
        let path = Element::empty(Tag::Path).with(
            "d",
            Value::Path(vec![
                Command::new('M', vec![0.0, 0.0]),
                Command {
                    letter: 'C',
                    groups: vec![vec![1.0, 2.0], vec![3.0, 4.0]],
                },
                Command::new('Z', vec![]),
            ]),
        );
        assert_eq!(path.to_svg(), "<path d=\"M 0 0 C 1 2, 3 4 Z\"/>");
    }
}

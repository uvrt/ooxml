The Rust page engine lays out and paints a text box holding an inline drawing that is not
a picture (a chart, a shape, SmartArt), as docx2svg does: the drawing is painted inside the
text box's drawing, after the box's pictures and before its text, and what its painter says
is said with the box's warnings. Such a document fell back to the Python layout before
(`support::TEXT_BOX`). A text box holding a drawing with text of its own, or a floating
drawing text wraps around, still falls back.

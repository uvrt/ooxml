//! docx-agent's styled-text reading, by example (`markdown/styled.py`'s cases).

use ooxml_markdown::styled::parse;

fn shape(text: &str) -> Vec<(String, bool, bool, Option<String>)> {
    parse(text)
        .pieces
        .into_iter()
        .map(|p| (p.text, p.strong, p.em, p.style))
        .collect()
}

fn t(text: &str, strong: bool, em: bool, style: Option<&str>) -> (String, bool, bool, Option<String>) {
    (text.to_owned(), strong, em, style.map(str::to_owned))
}

#[test]
fn emphasis_and_spans() {
    assert_eq!(
        shape("a **b** *c* d"),
        vec![
            t("a ", false, false, None),
            t("b", true, false, None),
            t(" ", false, false, None),
            t("c", false, true, None),
            t(" d", false, false, None)
        ]
    );
    assert_eq!(
        shape("5 * 3 and **open"),
        vec![t("5 * 3 and **open", false, false, None)]
    );
    assert_eq!(
        shape("file_name_here _x_"),
        vec![t("file_name_here _x_", false, false, None)]
    );
    assert_eq!(
        shape(r"\*not\* C:\Users"),
        vec![t(r"*not* C:\Users", false, false, None)]
    );
    assert_eq!(
        shape("see [the **term**]{Book Title} now"),
        vec![
            t("see ", false, false, None),
            t("the ", false, false, Some("Book Title")),
            t("term", true, false, Some("Book Title")),
            t(" now", false, false, None)
        ]
    );
    assert_eq!(
        shape("[x]{style=\"Quote Char\"}"),
        vec![t("x", false, false, Some("Quote Char"))]
    );
    assert_eq!(shape("[x]{custom-style='Q'}"), vec![t("x", false, false, Some("Q"))]);
    assert_eq!(shape("[x]{ }"), vec![t("[x]{ }", false, false, None)]);
    assert_eq!(shape("***both***"), vec![t("both", true, true, None)]);
    let styled = parse("a *b*");
    assert_eq!(styled.plain, "a b");
    assert_eq!((styled.pieces[1].start, styled.pieces[1].end), (2, 3));
}

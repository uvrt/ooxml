//! Against ooxml-edit's `chart_space` on the same data (the goldens: `golden/*.xml`, written
//! by it with the digest fixed at `AB` bytes, Word's look, `nl-NL`), and its refusals.

use super::{Labels, Legend, NewChart, NewKind, NewSeries, WORD_LOOK};
use ooxml_xml::Tree;
use std::sync::Arc;

fn series(name: &str, values: &[Option<f64>]) -> NewSeries {
    NewSeries {
        name: Some(name.to_owned()),
        values: values.to_vec(),
        line: None,
        size: None,
    }
}

fn two() -> Vec<NewSeries> {
    vec![
        series("2024", &[Some(1.0), Some(2.5), None]),
        series("2025", &[Some(3.0), Some(4.0), Some(5.0)]),
    ]
}

fn text() -> Labels {
    Labels::Text(["North", "South", "East"].iter().map(|s| (*s).to_owned()).collect())
}

fn chart(kind: NewKind, labels: Labels, series: Vec<NewSeries>) -> NewChart {
    NewChart {
        kind,
        labels,
        series,
        title: None,
        legend: Some(Legend::Default),
        number_format: None,
        look: WORD_LOOK,
        lang: "nl-NL".to_owned(),
        reversed: false,
        gridlines: true,
    }
}

fn written(chart: &NewChart) -> String {
    assert_eq!(chart.check(), Ok(()));
    let hash = |_: &[u8]| [0xAB; 32];
    let space = chart.chart_space("word/charts/chart1.xml", "rId1", &hash);
    let bytes = Tree::from_root(Arc::new(space)).write();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    text.split_once('\n').map(|(_, rest)| rest.to_owned()).unwrap_or(text)
}

fn cases() -> Vec<(&'static str, NewChart, &'static str)> {
    let mut column = chart(NewKind::Column, text(), two());
    column.title = Some("Sales\nby region".to_owned());
    let mut stacked = chart(NewKind::StackedBar, text(), two());
    stacked.legend = Some(Legend::Right);
    stacked.reversed = true;
    let mut bar = chart(NewKind::Bar, text(), two());
    bar.legend = None;
    bar.number_format = Some("#,##0.0".to_owned());
    let mut line = chart(NewKind::Line, text(), two());
    line.gridlines = false;
    let mut pie = chart(NewKind::Pie, text(), two().into_iter().take(1).collect());
    pie.title = Some("Share".to_owned());
    let mut scatter = chart(NewKind::Scatter, Labels::Numbers(vec![1.0, 2.5, 4.0]), two());
    scatter.reversed = true;
    let mut bubbles = series("a", &[Some(1.0), Some(2.0), Some(3.0)]);
    bubbles.size = Some(vec![Some(4.0), None, Some(6.0)]);
    let bubble = chart(NewKind::Bubble, Labels::Numbers(vec![1.0, 2.0, 3.0]), vec![bubbles]);
    let mut mixed = two();
    if let Some(last) = mixed.last_mut() {
        last.line = Some(true);
    }
    let mut combo = chart(NewKind::StackedColumn, text(), mixed);
    combo.reversed = true;
    combo.number_format = Some(String::new());
    vec![
        ("column", column, include_str!("golden/column.xml")),
        ("stacked_bar", stacked, include_str!("golden/stacked_bar.xml")),
        ("bar", bar, include_str!("golden/bar.xml")),
        ("line", line, include_str!("golden/line.xml")),
        ("pie", pie, include_str!("golden/pie.xml")),
        ("scatter", scatter, include_str!("golden/scatter.xml")),
        ("bubble", bubble, include_str!("golden/bubble.xml")),
        (
            "radar_filled",
            chart(NewKind::RadarFilled, text(), two()),
            include_str!("golden/radar_filled.xml"),
        ),
        (
            "radar",
            chart(NewKind::Radar, text(), two()),
            include_str!("golden/radar.xml"),
        ),
        (
            "dot",
            chart(NewKind::Dot, text(), two()),
            include_str!("golden/dot.xml"),
        ),
        ("combo", combo, include_str!("golden/combo.xml")),
    ]
}

#[test]
fn every_kind_written_as_ooxml_edit_writes_it() {
    for (name, chart, golden) in cases() {
        assert_eq!(written(&chart), golden, "{name}");
    }
}

#[test]
fn refusals_in_ooxml_edits_words() {
    let refused = |chart: &NewChart| chart.check().err().map(|e| e.0).unwrap_or_default();
    let mut short = two();
    short[0].values.pop();
    assert_eq!(
        refused(&chart(NewKind::Column, text(), short)),
        "series 0 has 2 values for 3 categories"
    );
    let mut lined = two();
    lined[1].line = Some(true);
    assert_eq!(
        refused(&chart(NewKind::StackedBar, text(), lined.clone())),
        "series 1: only a column chart draws a series as a line; PowerPoint cannot combine horizontal bars with a line"
    );
    assert_eq!(
        refused(&chart(NewKind::Line, text(), lined.clone())),
        "series 1: only a column chart draws a series as a line"
    );
    lined[0].line = Some(true);
    assert_eq!(
        refused(&chart(NewKind::Column, text(), lined)),
        "every series is a line: make it a line chart"
    );
    assert_eq!(
        refused(&chart(NewKind::Pie, text(), two())),
        "a pie chart shows one series"
    );
    assert_eq!(
        refused(&chart(NewKind::Column, text(), Vec::new())),
        "a chart needs at least one series"
    );
    assert_eq!(
        refused(&chart(NewKind::Column, Labels::Text(Vec::new()), two())),
        "a chart needs at least one category"
    );
    let mut sized = two();
    sized[1].size = Some(vec![Some(1.0)]);
    assert_eq!(
        refused(&chart(NewKind::Column, text(), sized)),
        "series 1: unknown key 'size'; a series is {'name', 'type', 'values'}"
    );
    let bubble = chart(NewKind::Bubble, Labels::Numbers(vec![1.0, 2.0, 3.0]), two());
    assert_eq!(
        refused(&bubble),
        "series 0: a bubble chart's series give size, one per point"
    );
    let mut reversed = chart(NewKind::Pie, text(), two().into_iter().take(1).collect());
    reversed.reversed = true;
    assert_eq!(refused(&reversed), "a pie chart has no category axis to reverse");
}

#[test]
fn bubble_sizes_beside_their_values() {
    let mut a = series("a", &[Some(1.0)]);
    a.size = Some(vec![Some(1.0)]);
    let bubble = chart(NewKind::Bubble, Labels::Numbers(vec![1.0]), vec![a.clone(), a]);
    assert_eq!(bubble.data_columns(), vec![(2, Some(3)), (4, Some(5))]);
    assert_eq!(
        chart(NewKind::Line, text(), two()).data_columns(),
        vec![(2, None), (3, None)]
    );
}

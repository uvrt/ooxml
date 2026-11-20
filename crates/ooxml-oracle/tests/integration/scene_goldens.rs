//! The scene and preset goldens, as a test: every scene ooxml-common's golden harness draws
//! -- the resolved slides frozen under `python/ooxml-common/tests/fixtures/scenes`, and the
//! hand-built scenes (`tests/data/handmade-scenes.json`, `python/scene_freeze.py`) -- and
//! the sheet of every preset geometry, painted by `ooxml-render` under PowerPoint's rules and
//! Word's, must equal its golden (`tests/golden/scenes`, `tests/golden/presets`) byte for
//! byte.

#[cfg(test)]
mod goldens {
    use ooxml_drawingml::drawable::{
        BodyProperties, Colour, Element, Fill, Geometry, Outline, Paragraph, ParagraphProperties, RunProperties, Shape,
        TextBody, TextRun, Transform,
    };
    use serde_json::Value;

    const COMMON: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../python/ooxml-common/tests");

    fn compare(name: &str, produced: &str, path: &str, failures: &mut Vec<String>) {
        let expected = std::fs::read_to_string(path).unwrap();
        if produced != expected {
            let at = produced
                .bytes()
                .zip(expected.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(produced.len().min(expected.len()));
            let from = at.saturating_sub(100);
            failures.push(format!(
                "{name} at byte {at}:\n  rust:   {}\n  python: {}",
                produced.get(from..(at + 150).min(produced.len())).unwrap_or("?"),
                expected.get(from..(at + 150).min(expected.len())).unwrap_or("?")
            ));
        }
    }

    fn scenes() -> Vec<(String, Value)> {
        let manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(format!("{COMMON}/fixtures/scenes.json")).unwrap()).unwrap();
        let mut out: Vec<(String, Value)> = manifest
            .as_object()
            .unwrap()
            .keys()
            .map(|name| {
                let data = std::fs::read_to_string(format!("{COMMON}/fixtures/scenes/{name}.json")).unwrap();
                (name.clone(), serde_json::from_str(&data).unwrap())
            })
            .collect();
        let handmade: Value = serde_json::from_str(include_str!("../data/handmade-scenes.json")).unwrap();
        for (name, scene) in handmade.as_object().unwrap() {
            out.push((name.clone(), scene.clone()));
        }
        out
    }

    #[test]
    fn every_scene_golden_is_drawn_byte_for_byte() {
        let mut failures = Vec::new();
        let mut count = 0;
        for (name, scene) in scenes() {
            let elements: Vec<Element> = scene["elements"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| ooxml_oracle::scene_data::element(e).unwrap())
                .collect();
            let (w, h) = (
                scene["width_emu"].as_f64().unwrap(),
                scene["height_emu"].as_f64().unwrap(),
            );
            for rules in ["powerpoint", "word"] {
                let produced = ooxml_oracle::drawing::document(&elements, w, h, rules, &[]);
                compare(
                    &format!("{name}-{rules}"),
                    &produced,
                    &format!("{COMMON}/golden/scenes/{name}-{rules}.svg"),
                    &mut failures,
                );
                count += 1;
            }
        }
        assert_eq!(count, 38, "{count} scene cases");
        assert!(
            failures.is_empty(),
            "{} of {count} differ:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// Each preset in a 1.2 x 0.75 inch box, eight to a row, its name as its text.
    fn preset_sheet() -> (Vec<Element>, f64, f64) {
        let (cell_w, cell_h, gap, per_row) = (1_097_280.0, 685_800.0, 182_880.0, 8usize);
        let mut names: Vec<String> = ooxml_drawingml::presets::presets()
            .iter()
            .map(|p| p.name.clone())
            .collect();
        names.push("bendUpArrow".into());
        names.sort();
        names.push("noSuchPreset".into());
        let elements = names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let (column, row) = ((index % per_row) as f64, (index / per_row) as f64);
                let body = TextBody {
                    paragraphs: vec![Paragraph {
                        runs: vec![TextRun {
                            text: name.clone(),
                            properties: RunProperties {
                                font_size: Some(7.0),
                                ..RunProperties::default()
                            },
                        }],
                        properties: ParagraphProperties::aligned("ctr"),
                        end_properties: None,
                    }],
                    properties: BodyProperties {
                        anchor: "ctr".into(),
                        ..BodyProperties::default()
                    },
                };
                Element::Shape(Box::new(Shape {
                    transform: Transform::of(
                        gap + column * (cell_w + gap),
                        gap + row * (cell_h + gap),
                        cell_w,
                        cell_h,
                    ),
                    geometry: Geometry::preset(name),
                    fill: Some(Fill::Solid(Colour::opaque("#DCE6F2"))),
                    outline: Some(Outline::solid(12700.0, Colour::opaque("#1F4E79"))),
                    text: Some(body),
                    ..Shape::default()
                }))
            })
            .collect();
        let rows = names.len().div_ceil(per_row) as f64;
        (
            elements,
            gap + per_row as f64 * (cell_w + gap),
            gap + rows * (cell_h + gap),
        )
    }

    #[test]
    fn every_preset_draws_as_its_golden() {
        let (elements, w, h) = preset_sheet();
        let mut failures = Vec::new();
        for rules in ["powerpoint", "word"] {
            let produced = ooxml_oracle::drawing::document(&elements, w, h, rules, &[]);
            compare(
                &format!("sheet-{rules}"),
                &produced,
                &format!("{COMMON}/golden/presets/sheet-{rules}.svg"),
                &mut failures,
            );
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}

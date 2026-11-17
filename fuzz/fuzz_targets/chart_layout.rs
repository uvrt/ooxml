//! Charts: no chart part panics the reader or the layout. Each input is parsed as XML, read
//! as a `c:chartSpace`, and laid out in two frames under PowerPoint's rules and Word's, with
//! fills and outlines resolved through an unthemed colour context.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ooxml_chart::layout::{chart_text_body, drawable_plots, lay_out, ChartInput, ChartStyle, Resolve};
use ooxml_chart::rules::{POWERPOINT, WORD};
use ooxml_colour::ColourContext;
use ooxml_drawingml::drawable::{Colour, Fill, Outline, Paragraph, ParagraphProperties, TextBody, TextRun};
use ooxml_drawingml::source;
use ooxml_xml::{parse, Limits};

struct Unthemed(ColourContext);

impl Resolve for Unthemed {
    fn fill(&self, fill: Option<&source::Fill>) -> Option<Fill> {
        match fill? {
            source::Fill::None => Some(Fill::None),
            source::Fill::Solid(c) => self.0.resolve(c).ok().flatten().map(|c| Fill::Solid(Colour::from(c))),
            _ => None,
        }
    }

    fn outline(&self, outline: Option<&source::Outline>) -> Option<Outline> {
        let outline = outline?;
        let fill = outline.fill.as_ref().and_then(|f| self.fill(Some(f)));
        if matches!(fill, Some(Fill::None)) {
            return None;
        }
        Some(Outline { fill, ..Outline::solid(outline.width.unwrap_or(9525.0), Colour::opaque("#000000")) })
    }

    fn text(&self, _rich: Option<&ooxml_drawingml::text::TextBody>, text: &str, size: f64, align: &str) -> TextBody {
        let mut run = TextRun { text: text.to_owned(), ..TextRun::default() };
        run.properties.font_size = Some(size);
        TextBody {
            paragraphs: vec![Paragraph {
                runs: vec![run],
                properties: ParagraphProperties::aligned(align),
                end_properties: None,
            }],
            properties: chart_text_body(),
        }
    }

    fn typeface(&self, face: &str) -> Option<String> {
        Some(face.to_owned())
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(tree) = parse(data, &Limits::default()) else { return };
    let Some(chart) = ooxml_chart::chart_space(tree.root()) else { return };
    let plots = drawable_plots(&chart);
    let Some(&plot) = plots.first() else { return };
    let resolve = Unthemed(ColourContext::unthemed());
    for (rules, (width, height)) in [(&POWERPOINT, (480.0, 288.0)), (&WORD, (90.0, 60.0))] {
        let style = ChartStyle {
            font_family: Some("Aptos".into()),
            font_size: 10.0,
            colour: Colour::opaque("#000000"),
            accents: vec![Colour::opaque("#4472c4"), Colour::opaque("#ed7d31")],
            font_family_ea: Some("MS Gothic".into()),
            office_2007: false,
            line_colour: None,
        };
        let input = ChartInput {
            chart: &chart,
            plot,
            plots: plots.clone(),
            width_pt: width,
            height_pt: height,
            style,
            resolve: &resolve,
            rules,
            locale: &ooxml_numfmt::EN_US,
            default_title: None,
        };
        let _ = lay_out(&input);
    }
});

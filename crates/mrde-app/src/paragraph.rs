//! Wrapped text whose layout height and painted lines come from the same wrap width.
//!
//! GPUI 0.2.2 text keeps the lines of its latest measurement, which may differ from the
//! measurement taffy used for the height. A long MR title then painted its second line
//! over the metadata below. This element shapes the text for the width it is measured
//! with and again for its final bounds, with a tolerance for layout rounding.
use gpui::*;

pub struct Paragraph {
    text: SharedString,
}

pub fn paragraph(text: impl Into<SharedString>) -> Paragraph {
    Paragraph { text: text.into() }
}

/// Layout rounds edges to whole pixels; one extra pixel keeps the measured and painted
/// line breaks identical.
fn wrap_width(width: Pixels) -> Pixels {
    px(f32::from(width).round() + 1.)
}

/// The inherited text style, read where it applies: during request layout or prepaint.
fn metrics(window: &Window) -> (TextStyle, Pixels, Pixels) {
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let line_height = style
        .line_height
        .to_pixels(font_size.into(), window.rem_size());
    (style, font_size, line_height)
}

fn shape(
    text: &SharedString,
    (style, font_size, line_height): &(TextStyle, Pixels, Pixels),
    width: Option<Pixels>,
    window: &mut Window,
) -> Vec<WrappedLine> {
    let _ = line_height;
    let runs = [style.to_run(text.len())];
    window
        .text_system()
        .shape_text(text.clone(), *font_size, &runs, width.map(wrap_width), None)
        .map(|lines| lines.into_iter().collect())
        .unwrap_or_default()
}

fn height(lines: &[WrappedLine], line_height: Pixels) -> Pixels {
    lines
        .iter()
        .map(|line| line.size(line_height).height)
        .fold(px(0.), |total, height| total + height)
}

impl IntoElement for Paragraph {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Paragraph {
    type RequestLayoutState = ();
    type PrepaintState = (Vec<WrappedLine>, Pixels);

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, ()) {
        let text = self.text.clone();
        let metrics = metrics(window);
        let line_height = metrics.2;
        let layout = Style {
            min_size: size(px(0.).into(), Length::Auto),
            flex_shrink: 1.,
            ..Default::default()
        };
        let id = window.request_measured_layout(layout, move |known, available, window, _| {
            if available.width == AvailableSpace::MinContent && known.width.is_none() {
                // Shrinkable: the parent decides the width, then the height follows.
                return size(px(0.), line_height);
            }
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let lines = shape(&text, &metrics, width, window);
            let widest = lines
                .iter()
                .map(|line| line.size(line_height).width)
                .fold(px(0.), |a, b| a.max(b));
            size(width.unwrap_or(widest), height(&lines, line_height))
        });
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
        let metrics = metrics(window);
        let line_height = metrics.2;
        let lines = shape(&self.text, &metrics, Some(bounds.size.width), window);
        (lines, line_height)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        (lines, line_height): &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut origin = bounds.origin;
        for line in lines.iter() {
            let _ = line.paint(origin, *line_height, TextAlign::Left, None, window, cx);
            origin.y += line.size(*line_height).height;
        }
    }
}

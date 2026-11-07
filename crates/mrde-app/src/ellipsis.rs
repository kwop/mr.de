//! Single-line text ending with "…" when it does not fit.
//!
//! GPUI 0.2.2's text element stores its layout from whichever measurement ran last, which
//! may be a layout probe at another width: `truncate()` then clips mid-glyph and
//! `line_clamp(1)` cuts after the first word. This element measures its full width for
//! layout, can shrink to zero, and truncates only at prepaint, against its final bounds.
use gpui::*;
use std::ops::Range;

pub struct Ellipsis {
    text: SharedString,
    colors: Vec<(Range<usize>, Hsla)>,
}

pub fn ellipsis(text: impl Into<SharedString>) -> Ellipsis {
    Ellipsis {
        text: text.into(),
        colors: Vec::new(),
    }
}

impl Ellipsis {
    /// Colors byte ranges of the text; ranges must be sorted and on char boundaries.
    pub fn colors(mut self, colors: impl IntoIterator<Item = (Range<usize>, Hsla)>) -> Self {
        self.colors = colors.into_iter().collect();
        self
    }

    fn runs(&self, style: &TextStyle) -> Vec<TextRun> {
        let mut runs = Vec::new();
        let mut cursor = 0;
        for (range, color) in &self.colors {
            if range.start > cursor {
                runs.push(style.to_run(range.start - cursor));
            }
            let mut run = style.to_run(range.len());
            run.color = *color;
            runs.push(run);
            cursor = range.end;
        }
        if cursor < self.text.len() {
            runs.push(style.to_run(self.text.len() - cursor));
        }
        runs
    }

    fn metrics(window: &Window) -> (TextStyle, Pixels, Pixels) {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = style
            .line_height
            .to_pixels(font_size.into(), window.rem_size());
        (style, font_size, line_height)
    }
}

impl IntoElement for Ellipsis {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Ellipsis {
    type RequestLayoutState = Pixels;
    type PrepaintState = Option<(ShapedLine, Pixels)>;

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
    ) -> (LayoutId, Pixels) {
        let (style, font_size, line_height) = Self::metrics(window);
        let full = window
            .text_system()
            .shape_line(self.text.clone(), font_size, &self.runs(&style), None)
            .width;
        let layout = Style {
            min_size: size(px(0.).into(), Length::Auto),
            flex_shrink: 1.,
            ..Default::default()
        };
        let id = window.request_measured_layout(layout, move |known, available, _, _| {
            let width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(limit) => full.min(limit),
                AvailableSpace::MinContent => px(0.),
                AvailableSpace::MaxContent => full,
            });
            size(width, line_height)
        });
        (id, full)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        full: &mut Pixels,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let (style, font_size, line_height) = Self::metrics(window);
        let mut runs = self.runs(&style);
        // Sub-pixel rounding must not ellipsize text that fits.
        let text = if *full > bounds.size.width + px(0.5) {
            cx.text_system()
                .line_wrapper(style.font(), font_size)
                .truncate_line(self.text.clone(), bounds.size.width, "…", &mut runs)
        } else {
            self.text.clone()
        };
        Some((
            window
                .text_system()
                .shape_line(text, font_size, &runs, None),
            line_height,
        ))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Pixels,
        line: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some((line, line_height)) = line {
            let _ = line.paint(bounds.origin, *line_height, window, cx);
        }
    }
}

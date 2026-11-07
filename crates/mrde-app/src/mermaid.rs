//! Mermaid diagrams inside Markdown, rendered to SVG in pure Rust (no browser or Node.js).
use crate::theme::*;
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId, Image,
    ImageFormat, InspectorElementId, LayoutId, Length, ObjectFit, Pixels, Style, Window, div, img,
    prelude::*, px, rgb, size,
};
use markdown::{ParseOptions, mdast::Node};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

/// Untrusted descriptions: bound the work done for a single diagram.
const MAX_SOURCE: usize = 20_000;
/// SVG images are rasterized at 1x by GPUI; render at 2x and display at 1x for Retina.
const SCALE: f32 = 2.;

#[derive(Debug, PartialEq)]
pub enum Segment {
    Markdown(String),
    Mermaid(String),
}

/// Splits top-level ```mermaid fences out of a Markdown document. Nested fences (inside
/// lists or quotes) and other code blocks stay Markdown.
pub fn split(source: &str) -> Vec<Segment> {
    if !source.contains("mermaid") {
        return vec![Segment::Markdown(source.to_owned())];
    }
    let Ok(Node::Root(root)) = markdown::to_mdast(source, &ParseOptions::gfm()) else {
        return vec![Segment::Markdown(source.to_owned())];
    };
    let mut segments = Vec::new();
    let mut cursor = 0;
    for node in &root.children {
        let Node::Code(code) = node else { continue };
        let is_mermaid = code
            .lang
            .as_deref()
            .is_some_and(|lang| lang.trim().eq_ignore_ascii_case("mermaid"));
        let Some(position) = node.position().filter(|_| is_mermaid) else {
            continue;
        };
        let before = &source[cursor..position.start.offset];
        if !before.trim().is_empty() {
            segments.push(Segment::Markdown(before.to_owned()));
        }
        segments.push(Segment::Mermaid(code.value.clone()));
        cursor = position.end.offset;
    }
    let rest = &source[cursor..];
    if !rest.trim().is_empty() || segments.is_empty() {
        segments.push(Segment::Markdown(rest.to_owned()));
    }
    segments
}

pub struct Diagram {
    pub image: Arc<Image>,
    pub width: f32,
    pub height: f32,
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

/// Logical size from the root `<svg>` element: width/height, else the viewBox.
fn dimensions(svg: &str) -> Option<(f32, f32, usize, usize)> {
    let start = svg.find("<svg")?;
    let end = svg[start..].find('>')? + start;
    let tag = &svg[start..end];
    let number = |value: String| value.trim_end_matches("px").trim().parse::<f32>().ok();
    let (width, height) = match (
        attribute(tag, "width").and_then(number),
        attribute(tag, "height").and_then(number),
    ) {
        (Some(w), Some(h)) => (w, h),
        _ => {
            let view_box = attribute(tag, "viewBox")?;
            let parts: Vec<f32> = view_box
                .split([' ', ','])
                .filter_map(|p| p.parse().ok())
                .collect();
            (*parts.get(2)?, *parts.get(3)?)
        }
    };
    (width > 0. && height > 0.).then_some((width, height, start, end))
}

/// Rewrites the root size so the image is rasterized at `SCALE`, keeping the viewBox.
fn scaled(svg: &str) -> Option<(String, f32, f32)> {
    let (width, height, start, end) = dimensions(svg)?;
    let mut tag = svg[start..end].to_owned();
    for name in ["width", "height"] {
        if let Some(value) = attribute(&tag, name) {
            tag = tag.replacen(&format!(" {name}=\"{value}\""), "", 1);
        }
    }
    if attribute(&tag, "viewBox").is_none() {
        tag.push_str(&format!(" viewBox=\"0 0 {width} {height}\""));
    }
    tag.push_str(&format!(
        " width=\"{}\" height=\"{}\"",
        width * SCALE,
        height * SCALE
    ));
    Some((
        format!("{}{}{}", &svg[..start], tag, &svg[end..]),
        width,
        height,
    ))
}

fn render_uncached(code: &str) -> Result<Diagram, String> {
    if code.len() > MAX_SOURCE {
        return Err("diagram is too large to render".into());
    }
    let mut theme = mermaid_rs_renderer::Theme::dark();
    theme.font_family = "Helvetica Neue, Helvetica, Arial, sans-serif".into();
    theme.background = "#212123".into();
    let options = mermaid_rs_renderer::RenderOptions {
        theme,
        layout: Default::default(),
    };
    let code = code.to_owned();
    let svg =
        std::panic::catch_unwind(move || mermaid_rs_renderer::render_with_options(&code, options))
            .map_err(|_| "the renderer stopped unexpectedly".to_string())?
            .map_err(|error| format!("{error:#}"))?;
    let (svg, width, height) = scaled(&svg).ok_or("the diagram has no size")?;
    Ok(Diagram {
        image: Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes())),
        width,
        height,
    })
}

type Cache = HashMap<String, Result<Arc<Diagram>, String>>;
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);

/// Rendering is cached by source so repainting never re-runs layout.
pub fn render(code: &str) -> Result<Arc<Diagram>, String> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(result) = cache.get(code) {
        return result.clone();
    }
    if cache.len() >= 64 {
        cache.clear();
    }
    let result = render_uncached(code).map(Arc::new);
    cache.insert(code.to_owned(), result.clone());
    result
}

/// An image filling the available width up to its natural size, at its aspect ratio.
///
/// A fixed width would become the minimum width of every ancestor and push the reading
/// column off screen, and GPUI 0.2.2's `img` replaces an `auto` height with the natural
/// height even when the width shrinks, leaving empty bands around a narrowed diagram.
/// This element sizes itself, then lays the image out at exactly that size.
struct Fitted {
    image: Arc<Image>,
    width: f32,
    height: f32,
}

impl IntoElement for Fitted {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Fitted {
    type RequestLayoutState = ();
    type PrepaintState = Option<AnyElement>;

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
        let natural = px(self.width);
        let ratio = self.width / self.height.max(1.);
        let layout = Style {
            min_size: size(px(0.).into(), Length::Auto),
            flex_shrink: 1.,
            ..Default::default()
        };
        let id = window.request_measured_layout(layout, move |known, available, _, _| {
            let width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(limit) => natural.min(limit),
                AvailableSpace::MinContent => px(0.),
                AvailableSpace::MaxContent => natural,
            });
            size(width, known.height.unwrap_or(width / ratio))
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
        cx: &mut App,
    ) -> Option<AnyElement> {
        let mut image = img(self.image.clone())
            .w(bounds.size.width)
            .h(bounds.size.height)
            .object_fit(ObjectFit::Contain)
            .into_any_element();
        image.prepaint_as_root(
            bounds.origin,
            bounds.size.map(AvailableSpace::Definite),
            window,
            cx,
        );
        Some(image)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        image: &mut Option<AnyElement>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(image) = image {
            image.paint(window, cx);
        }
    }
}

pub fn element(code: &str) -> AnyElement {
    match render(code) {
        Ok(diagram) => div()
            .debug_selector(|| "mermaid-diagram".into())
            .flex_shrink_0()
            .min_w_0()
            .p_3()
            .rounded(px(7.))
            .bg(rgb(FIELD))
            .flex()
            .justify_center()
            .child(Fitted {
                image: diagram.image.clone(),
                width: diagram.width,
                height: diagram.height,
            })
            .into_any_element(),
        Err(error) => div()
            .debug_selector(|| "mermaid-error".into())
            .flex_shrink_0()
            .w_full()
            .p_3()
            .rounded(px(7.))
            .bg(rgb(FIELD))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(WARNING))
                    .child(format!("Mermaid diagram could not be rendered: {error}")),
            )
            .child(
                div()
                    .font_family(CODE_FONT)
                    .text_size(px(CODE_SIZE))
                    .text_color(rgb(MUTED))
                    .child(code.to_owned()),
            )
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn splits_top_level_mermaid_fences_and_keeps_other_code() {
        let source = "# Flow\n\n```mermaid\nflowchart LR\n  A-->B\n```\n\nAfter\n\n```rust\nlet mermaid = 1;\n```\n\n- item\n\n  ```mermaid\n  graph TD; X-->Y\n  ```\n";
        let segments = split(source);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0], Segment::Markdown("# Flow\n\n".into()));
        assert_eq!(
            segments[1],
            Segment::Mermaid("flowchart LR\n  A-->B".into())
        );
        let Segment::Markdown(rest) = &segments[2] else {
            panic!("expected markdown")
        };
        assert!(rest.contains("```rust") && rest.contains("graph TD"));
        assert_eq!(
            split("plain text"),
            [Segment::Markdown("plain text".into())]
        );
    }
    #[test]
    fn renders_a_flowchart_to_a_retina_svg_and_reports_errors() {
        let diagram = render("flowchart LR\n  A[Start] --> B{Choice}\n  B --> C[Done]").unwrap();
        assert!(diagram.width > 0. && diagram.height > 0.);
        let svg = std::str::from_utf8(diagram.image.bytes()).unwrap();
        assert!(svg.contains(&format!("width=\"{}\"", diagram.width * SCALE)));
        assert!(render(&"x".repeat(MAX_SOURCE + 1)).is_err());
    }
    #[test]
    fn scaled_svg_keeps_view_box_and_doubles_size() {
        let (svg, w, h) = scaled(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100px\" height=\"50\"><g/></svg>",
        )
        .unwrap();
        assert_eq!((w, h), (100., 50.));
        assert!(
            svg.contains("viewBox=\"0 0 100 50\"") && svg.contains("width=\"200\" height=\"100\"")
        );
        assert!(scaled("<svg><g/></svg>").is_none());
    }
}

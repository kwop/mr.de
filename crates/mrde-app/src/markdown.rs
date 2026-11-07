//! Shared Markdown presentation for MR/issue descriptions and Claude prose.
use crate::theme::{FIELD, INK, LINE, UI_FONT, UI_SIZE};
use gpui::{
    AnyElement, App, ElementId, SharedString, StyleRefinement, Window, div, prelude::*, px, rems,
    rgb,
};
use gpui_component::{
    ActiveTheme,
    text::{TextView, TextViewStyle},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

enum Part {
    Markdown(SharedString),
    Mermaid(String),
}

struct Prepared {
    source: String,
    context: Option<(String, String)>,
    parts: Arc<[Part]>,
}

thread_local! {
    static PREPARED: RefCell<HashMap<u64, Prepared>> = RefCell::new(HashMap::new());
}

/// Image rewriting and Mermaid extraction parse the document, so they run once per
/// document: scrolling re-renders the whole view on every frame.
fn prepared(source: &str, context: Option<(&str, &str)>) -> Arc<[Part]> {
    let mut hasher = DefaultHasher::new();
    (source, context).hash(&mut hasher);
    let key = hasher.finish();
    let owned_context = || context.map(|(a, b)| (a.to_owned(), b.to_owned()));
    PREPARED.with_borrow_mut(|cache| {
        if let Some(entry) = cache.get(&key)
            && entry.source == source
            && entry.context == owned_context()
        {
            return entry.parts.clone();
        }
        if cache.len() >= 256 {
            cache.clear();
        }
        let parts: Arc<[Part]> = crate::mermaid::split(source)
            .into_iter()
            .map(|segment| match segment {
                crate::mermaid::Segment::Markdown(text) => {
                    Part::Markdown(prepare_images(&text, context).into())
                }
                crate::mermaid::Segment::Mermaid(code) => Part::Mermaid(code),
            })
            .collect();
        cache.insert(
            key,
            Prepared {
                source: source.to_owned(),
                context: owned_context(),
                parts: parts.clone(),
            },
        );
        parts
    })
}

fn prepared_async(
    source: &str,
    context: Option<(&str, &str)>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Result<Arc<Arc<[Part]>>, String>> {
    crate::deferred::load(
        format!("markdown:{context:?}:{source}").into(),
        window,
        cx,
        || {
            let source = source.to_owned();
            let context = context.map(|(a, b)| (a.to_owned(), b.to_owned()));
            move || {
                prepared(
                    &source,
                    context.as_ref().map(|(a, b)| (a.as_str(), b.as_str())),
                )
            }
        },
    )
}

/// Markdown with top-level ```mermaid blocks rendered as diagrams.
pub fn view(
    id: impl Into<SharedString>,
    source: &str,
    context: Option<(&str, &str)>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let id = id.into();
    let parts = match prepared_async(source, context, window, cx) {
        Some(Ok(parts)) => parts,
        None => {
            return div()
                .debug_selector(|| "markdown-loading".into())
                .py_3()
                .child("Preparing description…")
                .into_any_element();
        }
        Some(Err(error)) => return div().child(error).into_any_element(),
    };
    if let [Part::Markdown(text)] = &**parts {
        return render(id, text.clone(), window, cx).into_any_element();
    }
    let mut container = div()
        .w_full()
        .min_w_0()
        .h_auto()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap_3();
    for (index, part) in parts.iter().enumerate() {
        container = container.child(match part {
            Part::Markdown(text) => render(
                SharedString::from(format!("{id}-{index}")),
                text.clone(),
                window,
                cx,
            )
            .into_any_element(),
            Part::Mermaid(code) => crate::mermaid::element(code, window, cx),
        });
    }
    container.into_any_element()
}

/// One Markdown document, without Mermaid extraction. Image preparation is cached.
#[cfg(test)]
fn text_view(
    id: impl Into<ElementId>,
    source: &str,
    context: Option<(&str, &str)>,
    window: &mut Window,
    cx: &mut App,
) -> TextView {
    let text = match &*prepared(source, context) {
        [Part::Markdown(text)] => text.clone(),
        _ => prepare_images(source, context).into(),
    };
    render(id, text, window, cx)
}

fn render(
    id: impl Into<ElementId>,
    source: SharedString,
    window: &mut Window,
    cx: &mut App,
) -> TextView {
    let style = TextViewStyle {
        is_dark: true,
        highlight_theme: cx.theme().highlight_theme.clone(),
        // gpui-component 0.5 treats every root block as the last one when
        // scrollable(false), so paragraph_gap does not separate code and prose.
        // Give the code surface its own spacing, including inside lists/quotes.
        code_block: StyleRefinement::default()
            .bg(rgb(FIELD))
            .my(px(12.))
            .p(px(12.))
            .border_1()
            .border_color(rgb(LINE))
            .line_height(px(22.)),
        ..Default::default()
    }
    .paragraph_gap(rems(0.75))
    .heading_font_size(|level, _| {
        px(match level {
            1 => 22.,
            2 => 19.,
            3 => 16.,
            _ => 14.,
        })
    });
    // The containing pane owns scrolling. Each Markdown document sizes to its content.
    // Keys identify a document, so selecting another MR cannot reuse its rendered state.
    let large = source.len() > 16_384;
    TextView::markdown(id, source, window, cx)
        .style(style)
        .on_image_click(crate::image_viewer::open)
        .selectable(true)
        .scrollable(large)
        // TextView defaults to size_full(). Embedded documents must size to their
        // content and never squeeze alongside other blocks in a scrolling column.
        .h_auto()
        .when(large, |view| view.h(px(520.)))
        .flex_shrink_0()
        .w_full()
        .min_w_0()
        .font_family(UI_FONT)
        .text_size(px(UI_SIZE))
        .text_color(rgb(INK))
        .line_height(px(22.))
}

/// Rewrite only parsed image nodes, leaving code examples and the original domain
/// models untouched. Reference images become inline images for GPUI's renderer.
fn prepare_images(source: &str, context: Option<(&str, &str)>) -> String {
    use markdown::{ParseOptions, mdast::Node};
    use std::collections::HashMap;
    if !source.contains("![") {
        return source.to_owned();
    }
    let Ok(tree) = markdown::to_mdast(source, &ParseOptions::gfm()) else {
        return source.to_owned();
    };
    fn definitions<'a>(node: &'a Node, result: &mut HashMap<&'a str, &'a str>) {
        if let Node::Definition(def) = node {
            result.entry(&def.identifier).or_insert(&def.url);
        }
        if let Some(children) = node.children() {
            for child in children {
                definitions(child, result);
            }
        }
    }
    fn escape(text: &str) -> String {
        text.replace('\\', "\\\\")
            .replace('[', "\\[")
            .replace(']', "\\]")
    }
    fn replacements(
        node: &Node,
        defs: &HashMap<&str, &str>,
        context: Option<(&str, &str)>,
        edits: &mut Vec<(std::ops::Range<usize>, String)>,
    ) {
        let image = match node {
            Node::Image(image) => Some((image.url.as_str(), image.alt.as_str())),
            Node::ImageReference(image) => defs
                .get(image.identifier.as_str())
                .map(|url| (*url, image.alt.as_str())),
            _ => None,
        };
        if let Some((url, alt)) = image
            && let Some(position) = node.position()
        {
            let resolved = context.and_then(|(instance, document)| {
                mrde_forge::uploads::resolve_image_url(instance, document, url)
            });
            let replacement = match resolved {
                Some(url) => format!("![{}](<{url}>)", escape(alt)),
                None => format!("Image unavailable: {}", escape(alt)),
            };
            edits.push((position.start.offset..position.end.offset, replacement));
        }
        if let Some(children) = node.children() {
            for child in children {
                replacements(child, defs, context, edits);
            }
        }
    }
    let mut defs = HashMap::new();
    definitions(&tree, &mut defs);
    let mut edits = Vec::new();
    replacements(&tree, &defs, context, &mut edits);
    let mut prepared = source.to_owned();
    for (range, text) in edits.into_iter().rev() {
        prepared.replace_range(range, &text);
    }
    prepared
}

#[cfg(test)]
mod tests {
    use super::*;
    const UPLOAD: &str = "/uploads/5f4dcc3b5aa765d61d8327deb882cf99/01-schedule-sheet.png";
    #[test]
    fn prepares_inline_and_reference_images_without_rewriting_code() {
        let source = format!(
            "![Sheet]({UPLOAD})\n\n![Second][sheet]\n\n[sheet]: {UPLOAD}\n\n`![Code]({UPLOAD})`\n\n```md\n![Example]({UPLOAD})\n```\n"
        );
        let result = prepare_images(
            &source,
            Some((
                "https://gitlab.example.com",
                "https://gitlab.example.com/team/app/-/merge_requests/7",
            )),
        );
        let expected = "https://gitlab.example.com/api/v4/projects/team%2Fapp/uploads/5f4dcc3b5aa765d61d8327deb882cf99/01-schedule-sheet.png";
        assert!(result.starts_with(&format!("![Sheet](<{expected}>)")));
        assert!(result.contains(&format!("![Second](<{expected}>)")));
        assert!(result.contains(&format!("`![Code]({UPLOAD})`")));
        assert!(result.contains(&format!("```md\n![Example]({UPLOAD})\n```")));
    }
    #[test]
    fn unsupported_image_sources_are_not_loaded_as_local_assets() {
        let result = prepare_images(
            "![Private](file:///etc/passwd)",
            Some((
                "https://gitlab.example.com",
                "https://gitlab.example.com/team/app/-/issues/1",
            )),
        );
        assert_eq!(result, "Image unavailable: Private");
    }
    #[test]
    fn documents_are_prepared_once_per_source_and_context() {
        let source = &format!("Text ![shot]({UPLOAD})\n\n```mermaid\ngraph TD; A-->B\n```\n");
        let context = Some((
            "https://gitlab.example.com",
            "https://gitlab.example.com/t/a/-/issues/1",
        ));
        let first = prepared(source, context);
        assert!(
            Arc::ptr_eq(&first, &prepared(source, context)),
            "scrolling must not re-parse"
        );
        assert!(!Arc::ptr_eq(&first, &prepared(source, None)));
        assert!(
            matches!(&*first, [Part::Markdown(text), Part::Mermaid(_)] if text.contains("/api/v4/projects/"))
        );
    }
}

#[cfg(test)]
#[path = "markdown_layout_tests.rs"]
mod layout_tests;

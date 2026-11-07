//! A window-sized image overlay that reuses the authenticated GPUI image cache.
use crate::theme::{HEADING, INFO, INK, MUTED, OneLine, PAGE, PANEL_LINE, UI_FONT};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName,
    scroll::{Scrollbar, ScrollbarShow},
};
use std::collections::HashMap;

actions!(image_viewer, [CloseImage, KeepImageFocus]);
const CONTEXT: &str = "MrdeImageViewer";

#[derive(Default)]
struct Viewers(HashMap<WindowId, WeakEntity<ImageViewer>>);
impl Global for Viewers {}

pub(crate) struct ImageViewer {
    image: Option<(SharedUri, String)>,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    zoom: f32,
    scroll: ScrollHandle,
    fitted: Size<Pixels>,
}

pub(crate) fn install(window: &mut Window, cx: &mut App) -> Entity<ImageViewer> {
    if !cx.has_global::<Viewers>() {
        cx.set_global(Viewers::default());
        cx.bind_keys([
            KeyBinding::new("escape", CloseImage, Some(CONTEXT)),
            KeyBinding::new("enter", CloseImage, Some(CONTEXT)),
            KeyBinding::new("space", CloseImage, Some(CONTEXT)),
            KeyBinding::new("tab", KeepImageFocus, Some(CONTEXT)),
            KeyBinding::new("shift-tab", KeepImageFocus, Some(CONTEXT)),
        ]);
    }
    let viewer = cx.new(|cx| ImageViewer {
        image: None,
        focus: cx.focus_handle(),
        previous_focus: None,
        zoom: 1.,
        scroll: ScrollHandle::new(),
        fitted: Size::default(),
    });
    let viewers = &mut cx.global_mut::<Viewers>().0;
    viewers.retain(|_, viewer| viewer.upgrade().is_some());
    viewers.insert(window.window_handle().window_id(), viewer.downgrade());
    viewer
}

pub(crate) fn open(uri: SharedUri, title: String, window: &mut Window, cx: &mut App) {
    let viewer = cx
        .try_global::<Viewers>()
        .and_then(|viewers| viewers.0.get(&window.window_handle().window_id()))
        .and_then(WeakEntity::upgrade);
    if let Some(viewer) = viewer {
        viewer.update(cx, |this, cx| {
            if this.image.is_none() {
                this.previous_focus = window.focused(cx);
            }
            this.image = Some((uri, title));
            this.zoom = 1.;
            this.scroll.set_offset(point(px(0.), px(0.)));
            this.focus.focus(window);
            cx.notify();
        });
    }
}

impl ImageViewer {
    /// Keep the same point beneath the viewport center when changing magnification.
    pub(crate) fn set_zoom(&mut self, zoom: f32, window: &Window, cx: &mut Context<Self>) {
        let zoom = zoom.clamp(1., 8.);
        let viewport = window.viewport_size() - size(px(0.), px(52.));
        let old_image = self.fitted.map(|value| value * self.zoom);
        let new_image = self.fitted.map(|value| value * zoom);
        let old_content = (old_image + size(px(24.), px(24.))).max(&viewport);
        let new_content = (new_image + size(px(24.), px(24.))).max(&viewport);
        let old_offset = self.scroll.offset();
        let axis = |view: Pixels,
                    old: Pixels,
                    new: Pixels,
                    before: Pixels,
                    after: Pixels,
                    offset: Pixels| {
            let point = (view / 2. - offset - (before - old) / 2.) / old.max(px(1.));
            (view / 2. - (after - new) / 2. - new * point).clamp(view - after, px(0.))
        };
        self.scroll.set_offset(point(
            axis(
                viewport.width,
                old_image.width,
                new_image.width,
                old_content.width,
                new_content.width,
                old_offset.x,
            ),
            axis(
                viewport.height,
                old_image.height,
                new_image.height,
                old_content.height,
                new_content.height,
                old_offset.y,
            ),
        ));
        self.zoom = zoom;
        cx.notify();
    }
    #[cfg(test)]
    pub(crate) fn source(&self) -> Option<&SharedUri> {
        self.image.as_ref().map(|(uri, _)| uri)
    }
    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.image = None;
        if let Some(focus) = self.previous_focus.take() {
            focus.focus(window);
        } else {
            window.blur();
        }
        cx.notify();
    }
}

impl Render for ImageViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((uri, title)) = &self.image else {
            return div().absolute().size(px(0.)).into_any_element();
        };
        let viewport = window.viewport_size();
        let width = (f32::from(viewport.width) - 24.).max(1.);
        let height = (f32::from(viewport.height) - 76.).max(1.);
        let natural = window
            .use_asset::<ImgResourceLoader>(&Resource::Uri(uri.clone()), cx)
            .and_then(Result::ok)
            .map(|image| {
                let dimensions = image.size(0);
                size(
                    px(dimensions.width.0 as f32),
                    px(dimensions.height.0 as f32),
                )
            })
            .unwrap_or(size(px(width), px(height)));
        let scale =
            (px(width) / natural.width.max(px(1.))).min(px(height) / natural.height.max(px(1.)));
        self.fitted = natural.map(|value| value * scale);
        let image_size = self.fitted.map(|value| value * self.zoom);
        let content_size = (image_size + size(px(24.), px(24.)))
            .max(&size(viewport.width, viewport.height - px(52.)));
        let control = |id: &'static str, label: String| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .cursor_pointer()
                .px_3()
                .py_1()
                .rounded(px(6.))
                .border_1()
                .border_color(rgb(PANEL_LINE))
                .text_size(px(12.))
                .flex()
                .items_center()
                .gap_2()
                .child(label)
        };
        div()
            .id("image-viewer")
            .debug_selector(|| "image-viewer".into())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(rgb(PAGE))
            .text_color(rgb(INK))
            .font_family(UI_FONT)
            .key_context(CONTEXT)
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, _: &CloseImage, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &KeepImageFocus, window, cx| {
                this.focus.focus(window);
                cx.stop_propagation();
            }))
            .on_click(|_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(52.))
                    .px_4()
                    .when(cfg!(target_os = "macos"), |bar| bar.pl(px(100.)))
                    .flex()
                    .items_center()
                    .gap_4()
                    .border_b_1()
                    .border_color(rgb(PANEL_LINE))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .one_line()
                            .text_size(px(13.))
                            .child(crate::ellipsis::ellipsis(if title.is_empty() {
                                "Image".to_string()
                            } else {
                                title.clone()
                            })),
                    )
                    .child(control("image-fit", "Fit".into()).on_click(cx.listener(
                        |this, _, window, cx| {
                            this.set_zoom(1., window, cx);
                        },
                    )))
                    .child(control("image-zoom-out", "−".into()).on_click(cx.listener(
                        |this, _, window, cx| {
                            this.set_zoom(this.zoom / 1.5, window, cx);
                        },
                    )))
                    .child(
                        control("image-zoom-in", format!("Zoom {:.0}%", self.zoom * 100.))
                            .child(Icon::new(IconName::Search).size(px(14.)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_zoom(this.zoom * 1.5, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child("Esc to close"),
                    )
                    .child(
                        div()
                            .id("close-image")
                            .debug_selector(|| "close-image".into())
                            .track_focus(&self.focus)
                            .cursor_pointer()
                            .h(px(32.))
                            .px_3()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(rgb(INFO))
                            .text_color(rgb(HEADING))
                            .text_size(px(13.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(IconName::Close).size(px(14.)))
                            .child("Close")
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.close(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        div()
                            .id("image-viewport")
                            .debug_selector(|| "image-viewer-canvas".into())
                            .track_scroll(&self.scroll)
                            .overflow_scroll()
                            .size_full()
                            .min_h_0()
                            .min_w_0()
                            .w_full()
                            .child(
                                div()
                                    .debug_selector(|| "image-zoom-content".into())
                                    .w(content_size.width)
                                    .h(content_size.height)
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        img(uri.clone())
                                            .debug_selector(|| "image-displayed".into())
                                            .flex_shrink_0()
                                            .w(image_size.width)
                                            .h(image_size.height)
                                            .object_fit(ObjectFit::Contain)
                                            .with_loading(|| {
                                                div()
                                                    .size_full()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .text_color(rgb(MUTED))
                                                    .child("Loading image…")
                                                    .into_any_element()
                                            })
                                            .with_fallback(|| {
                                                div()
                                                    .debug_selector(|| "image-viewer-error".into())
                                                    .size_full()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .text_color(rgb(MUTED))
                                                    .child("Could not load this image.")
                                                    .into_any_element()
                                            }),
                                    ),
                            ),
                    )
                    .child(Scrollbar::new(&self.scroll).scrollbar_show(ScrollbarShow::Always)),
            )
            .into_any_element()
    }
}

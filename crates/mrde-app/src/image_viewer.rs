//! A window-sized image overlay that reuses the authenticated GPUI image cache.
use crate::theme::{HEADING, INFO, INK, MUTED, OneLine, PAGE, PANEL_LINE, UI_FONT};
use gpui::{prelude::*, *};
use gpui_component::{Icon, IconName};
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
            this.focus.focus(window);
            cx.notify();
        });
    }
}

impl ImageViewer {
    #[cfg(test)]
    pub(crate) fn source(&self) -> Option<&SharedUri> {
        self.image.as_ref().map(|(uri, _)| uri)
    }
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((uri, title)) = &self.image else {
            return div().absolute().size(px(0.)).into_any_element();
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
                    .debug_selector(|| "image-viewer-canvas".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .p_3()
                    .child(
                        img(uri.clone())
                            .size_full()
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
            )
            .into_any_element()
    }
}

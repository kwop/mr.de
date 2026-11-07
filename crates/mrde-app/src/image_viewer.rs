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
const TOOLBAR_HEIGHT: f32 = 52.;
const MAX_ZOOM: f32 = 8.;
const DRAG_THRESHOLD: f32 = 4.;

struct Pan {
    start: Point<Pixels>,
    offset: Point<Pixels>,
    moved: bool,
}

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
    ready: bool,
    pointer: Option<Point<Pixels>>,
    pan: Option<Pan>,
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
        ready: false,
        pointer: None,
        pan: None,
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
            this.ready = false;
            this.pointer = None;
            this.pan = None;
            this.scroll.set_offset(point(px(0.), px(0.)));
            this.focus.focus(window);
            cx.notify();
        });
    }
}

impl ImageViewer {
    fn viewport(window: &Window) -> Size<Pixels> {
        (window.viewport_size() - size(px(0.), px(TOOLBAR_HEIGHT))).max(&size(px(1.), px(1.)))
    }
    fn content_size(&self, viewport: Size<Pixels>) -> Size<Pixels> {
        (self.fitted.map(|v| v * self.zoom) + size(px(24.), px(24.))).max(&viewport)
    }
    fn image_bounds(&self, window: &Window) -> Bounds<Pixels> {
        let image = self.fitted.map(|v| v * self.zoom);
        let content = self.content_size(Self::viewport(window));
        Bounds::new(
            point(
                (content.width - image.width) / 2.,
                (content.height - image.height) / 2. + px(TOOLBAR_HEIGHT),
            ) + self.scroll.offset(),
            image,
        )
    }
    fn pointer_over_image(&self, position: Point<Pixels>, window: &Window) -> bool {
        self.ready
            && Bounds::new(point(px(0.), px(TOOLBAR_HEIGHT)), Self::viewport(window))
                .contains(&position)
            && self.image_bounds(window).contains(&position)
    }
    /// Toolbar controls preserve the viewport center; image clicks preserve their focal point.
    pub(crate) fn set_zoom(&mut self, zoom: f32, window: &Window, cx: &mut Context<Self>) {
        let viewport = Self::viewport(window);
        self.zoom_at(
            zoom,
            point(
                viewport.width / 2.,
                viewport.height / 2. + px(TOOLBAR_HEIGHT),
            ),
            window,
            cx,
        );
    }
    fn zoom_at(
        &mut self,
        zoom: f32,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let zoom = zoom.clamp(1., MAX_ZOOM);
        let viewport = Self::viewport(window);
        let old_image = self.fitted.map(|value| value * self.zoom);
        let new_image = self.fitted.map(|value| value * zoom);
        let old_content = self.content_size(viewport);
        let new_content = (new_image + size(px(24.), px(24.))).max(&viewport);
        let old_offset = self.scroll.offset();
        let axis = |view: Pixels,
                    focal: Pixels,
                    old: Pixels,
                    new: Pixels,
                    before: Pixels,
                    after: Pixels,
                    offset: Pixels| {
            let relative = (focal - offset - (before - old) / 2.) / old.max(px(1.));
            (focal - (after - new) / 2. - new * relative).clamp(view - after, px(0.))
        };
        self.scroll.set_offset(point(
            axis(
                viewport.width,
                position.x,
                old_image.width,
                new_image.width,
                old_content.width,
                new_content.width,
                old_offset.x,
            ),
            axis(
                viewport.height,
                position.y - px(TOOLBAR_HEIGHT),
                old_image.height,
                new_image.height,
                old_content.height,
                new_content.height,
                old_offset.y,
            ),
        ));
        self.zoom = zoom;
        self.pan = None;
        cx.notify();
    }
    fn next_zoom(&self) -> f32 {
        if self.zoom >= MAX_ZOOM {
            1.
        } else {
            (self.zoom * 2.).min(MAX_ZOOM)
        }
    }
    pub(crate) fn pointer_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let previous_pointer = self.pointer;
        let had_pan = self.pan.is_some();
        // Recover if mouse-up was delivered outside the window or the app lost capture.
        if event.pressed_button != Some(MouseButton::Left) {
            self.pan = None;
        }
        let viewport = Self::viewport(window);
        let content = self.content_size(viewport);
        if let Some(pan) = &mut self.pan {
            let delta = event.position - pan.start;
            pan.moved |= delta.x.abs() > px(DRAG_THRESHOLD) || delta.y.abs() > px(DRAG_THRESHOLD);
            if pan.moved {
                let offset = pan.offset + delta;
                self.scroll.set_offset(point(
                    offset.x.clamp(viewport.width - content.width, px(0.)),
                    offset.y.clamp(viewport.height - content.height, px(0.)),
                ));
            }
        }
        self.pointer = self
            .pointer_over_image(event.position, window)
            .then_some(event.position);
        if self.pointer != previous_pointer || had_pan {
            cx.notify();
        }
    }
    fn pointer_up(&mut self, event: &MouseUpEvent, window: &Window, cx: &mut Context<Self>) {
        if event.button != MouseButton::Left {
            return;
        }
        let Some(pan) = self.pan.take() else {
            return;
        };
        let delta = event.position - pan.start;
        if !pan.moved
            && delta.x.abs() <= px(DRAG_THRESHOLD)
            && delta.y.abs() <= px(DRAG_THRESHOLD)
            && self.pointer_over_image(event.position, window)
        {
            self.zoom_at(self.next_zoom(), event.position, window, cx);
        }
        self.pointer = self
            .pointer_over_image(event.position, window)
            .then_some(event.position);
        cx.stop_propagation();
        cx.notify();
    }
    #[cfg(test)]
    pub(crate) fn source(&self) -> Option<&SharedUri> {
        self.image.as_ref().map(|(uri, _)| uri)
    }
    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.image = None;
        self.pointer = None;
        self.pan = None;
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
        let asset = window
            .use_asset::<ImgResourceLoader>(&Resource::Uri(uri.clone()), cx)
            .and_then(Result::ok);
        self.ready = asset.is_some();
        let natural = asset
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
        let content_size = self.content_size(Self::viewport(window));
        let dragging = self.pan.as_ref().is_some_and(|pan| pan.moved);
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
            .when(dragging, |layer| layer.cursor(CursorStyle::ClosedHand))
            .on_click(|_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(TOOLBAR_HEIGHT))
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
                            this.set_zoom(this.zoom / 2., window, cx);
                        },
                    )))
                    .child(
                        control("image-zoom-in", format!("Zoom {:.0}%", self.zoom * 100.))
                            .child(Icon::new(IconName::Search).size(px(14.)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_zoom(this.next_zoom(), window, cx);
                            })),
                    )
                    .child(div().text_size(px(12.)).text_color(rgb(MUTED)).child(
                        if self.zoom >= MAX_ZOOM {
                            "Click to fit · Drag to move · Esc to close"
                        } else {
                            "Click to zoom · Drag to move · Esc to close"
                        },
                    ))
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
                                        div()
                                            .id("image-interaction")
                                            .flex_shrink_0()
                                            .w(image_size.width)
                                            .h(image_size.height)
                                            .cursor(if dragging {
                                                CursorStyle::ClosedHand
                                            } else if self.ready {
                                                CursorStyle::None
                                            } else {
                                                CursorStyle::Arrow
                                            })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(
                                                    |this, event: &MouseDownEvent, _, cx| {
                                                        if this.ready {
                                                            this.pan = Some(Pan {
                                                                start: event.position,
                                                                offset: this.scroll.offset(),
                                                                moved: false,
                                                            });
                                                            cx.stop_propagation();
                                                            cx.notify();
                                                        }
                                                    },
                                                ),
                                            )
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
                                                            .debug_selector(|| {
                                                                "image-viewer-error".into()
                                                            })
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
                            ),
                    )
                    .child(Scrollbar::new(&self.scroll).scrollbar_show(ScrollbarShow::Always)),
            )
            .child(input_layer(cx))
            .when(!dragging, |layer| {
                layer.children(
                    self.pointer
                        .filter(|position| self.pointer_over_image(*position, window))
                        .map(|position| magnifier(position, self.zoom >= MAX_ZOOM)),
                )
            })
            .into_any_element()
    }
}

/// GPUI 0.2.2 has no magnifier cursor. Draw one above the image without a hitbox,
/// with a contrasting outline that stays visible on both white and dark images.
fn magnifier(position: Point<Pixels>, reset: bool) -> impl IntoElement {
    div()
        .debug_selector(move || {
            if reset {
                "image-reset-cursor"
            } else {
                "image-zoom-cursor"
            }
            .into()
        })
        .absolute()
        .left(position.x - px(8.))
        .top(position.y - px(8.))
        .size(px(24.))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let at = |x, y| bounds.origin + point(px(x), px(y));
                    for (width, color) in [(4., PAGE), (2., HEADING)] {
                        let mut path = PathBuilder::stroke(px(width));
                        path.move_to(at(15., 8.));
                        path.arc_to(point(px(7.), px(7.)), px(0.), false, true, at(1., 8.));
                        path.arc_to(point(px(7.), px(7.)), px(0.), false, true, at(15., 8.));
                        path.close();
                        path.move_to(at(13., 13.));
                        path.line_to(at(21., 21.));
                        path.move_to(at(4., 8.));
                        path.line_to(at(12., 8.));
                        if !reset {
                            path.move_to(at(8., 4.));
                            path.line_to(at(8., 12.));
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, rgb(color));
                        }
                    }
                },
            )
            .size_full(),
        )
}

/// Track the gesture at window level so crossing the image or window edge cannot
/// lose a mouse-up. A canvas registers input listeners without intercepting hit tests.
fn input_layer(cx: &mut Context<ImageViewer>) -> impl IntoElement {
    let motion = cx.listener(|this, event: &MouseMoveEvent, window, cx| {
        this.pointer_move(event, window, cx);
    });
    let release = cx.listener(|this, event: &MouseUpEvent, window, cx| {
        this.pointer_up(event, window, cx);
    });
    let exit = cx.listener(|this, _: &MouseExitEvent, _, cx| {
        this.pointer = None;
        cx.notify();
    });
    canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase.bubble() {
                    motion(event, window, cx);
                }
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase.capture() {
                    release(event, window, cx);
                }
            });
            window.on_mouse_event(move |event: &MouseExitEvent, phase, window, cx| {
                if phase.bubble() {
                    exit(event, window, cx);
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

mod backend;
mod deferred;
mod demo;
mod ellipsis;
mod image_viewer;
mod images;
mod markdown;
mod mermaid;
mod paragraph;
mod projects;
mod settings;
#[cfg(target_os = "macos")]
mod snapshot;
#[cfg(target_os = "macos")]
mod terminal;
mod theme;
mod threads;
mod ui;

use gpui::{prelude::*, *};
use gpui_component::Root;

actions!(mrde, [Quit]);

pub fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let runtime = std::sync::Arc::new(tokio::runtime::Runtime::new()?);
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            theme::init(cx);
            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            #[cfg(target_os = "macos")]
            let (width, height) = snapshot::window_size().unwrap_or((1280., 860.));
            #[cfg(not(target_os = "macos"))]
            let (width, height) = (1280., 860.);
            let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
            if let Err(error) = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(1100.), px(720.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("mr.de".into()),
                        appears_transparent: true,
                        // Centered in the 52 pt title bar (`theme::TITLEBAR_HEIGHT`).
                        traffic_light_position: Some(point(px(18.), px(19.))),
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| ui::Cockpit::new(runtime, window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            ) {
                tracing::error!(%error, "Failed to open window");
            }
            cx.activate(true);
            #[cfg(target_os = "macos")]
            if let Some(path) = snapshot::requested() {
                snapshot::schedule(path, cx);
            }
        });
    Ok(())
}

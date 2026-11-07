//! CPU work belongs on GPUI's background executor, never in layout or paint.
use gpui::{App, AppContext, SharedString, Task, Window};
use std::sync::Arc;

struct Deferred<T> {
    result: Option<Result<Arc<T>, String>>,
    _task: Task<()>,
}

/// Window-scoped memo. The exact key must include every input affecting the result.
/// Dropping an unused state drops its task; a late completion cannot update a new key.
/// The factory runs only on a cache miss, so large inputs are not cloned per frame.
pub fn load<T, F>(
    key: SharedString,
    window: &mut Window,
    cx: &mut App,
    factory: impl FnOnce() -> F,
) -> Option<Result<Arc<T>, String>>
where
    T: Send + Sync + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let parent = window.current_view();
    let state = window.use_keyed_state(key, cx, |_, cx| {
        let compute = factory();
        let work = cx.background_spawn(async move {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(compute))
                .map(Arc::new)
                .map_err(|_| "Background rendering failed. Reopen the view to retry.".to_string())
        });
        let task = cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |state: &mut Deferred<T>, cx| {
                state.result = Some(result);
                cx.notify();
                let app: &mut App = cx;
                app.notify(parent);
            });
        });
        Deferred {
            result: None,
            _task: task,
        }
    });
    state.read(cx).result.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Context, IntoElement, Render, TestAppContext, VisualTestContext, div, point, prelude::*,
        px, size,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture {
        key: usize,
        result: Option<usize>,
        calls: Arc<AtomicUsize>,
    }
    impl Render for Fixture {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let key = self.key;
            let calls = self.calls.clone();
            let before = calls.load(Ordering::SeqCst);
            let value = load(format!("test:{key}").into(), window, cx, || {
                move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    key
                }
            });
            assert_eq!(
                self.calls.load(Ordering::SeqCst),
                before,
                "Rendering must never execute the computation inline"
            );
            self.result = value.map(|v| *v.unwrap());
            div().child(format!("{:?}", self.result))
        }
    }
    #[gpui::test]
    fn work_is_deferred_cached_and_isolated_when_the_key_changes(cx: &mut TestAppContext) {
        let calls = Arc::new(AtomicUsize::new(0));
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::init(cx);
        });
        let mut fixture = None;
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|_| Fixture {
                key: 1,
                result: None,
                calls: calls.clone(),
            });
            fixture = Some(view.clone());
            gpui_component::Root::new(view, window, cx)
        });
        let root = window.root(cx).unwrap();
        let view = fixture.unwrap();
        let mut visual = VisualTestContext::from_window(*window, cx);
        let draw = |cx: &mut VisualTestContext| {
            cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, _| {
                root.clone()
            });
        };
        draw(&mut visual);
        visual.update(|_, cx| {
            view.update(cx, |v, cx| {
                v.key = 2;
                cx.notify();
            })
        });
        for _ in 0..3 {
            draw(&mut visual);
            visual.run_until_parked();
        }
        assert_eq!(visual.update(|_, cx| view.read(cx).result), Some(2));
        let count = calls.load(Ordering::SeqCst);
        draw(&mut visual);
        visual.run_until_parked();
        assert_eq!(calls.load(Ordering::SeqCst), count);
    }
}

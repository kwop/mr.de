use super::*;
use mrde_core::resolver::ResolverState;

#[gpui::test]
fn closing_a_thread_stops_it_rejects_late_events_and_reopens_without_running(
    cx: &mut TestAppContext,
) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![queue_fixture(1, "2026-11-05T12:00:00Z")],
        resolver_gate: Some(tokio::sync::Semaphore::new(0)),
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        queue_config(),
    );
    let cx = &mut visual;
    click("mr-1-1", cx);
    finish(&view, &root, cx);
    click("super-resolver", cx);
    pump_threads(&view, &root, cx);
    let mut late = executor.resolver_runs.lock().unwrap()[0].clone();
    let id = late.id.clone();
    click("close-thread", cx);
    late.state = ResolverState::Succeeded;
    late.sequence += 100;
    executor.resolver_updates.lock().unwrap()[0](mrde_resolver::Update::Thread(Box::new(late)));
    pump_threads(&view, &root, cx);
    let closed = cx.update(|_, cx| view.read(cx).threads.read(cx).records[0].clone());
    assert!(closed.closed_at.is_some());
    assert_eq!(closed.state, ResolverState::Stopped);
    assert!(!closed.sessions.is_empty());
    assert!(closed.sessions[0].finished_at.is_some());
    assert_eq!(executor.resolver_runs.lock().unwrap().len(), 1);
    click("toggle-closed-threads", cx);
    draw(&root, cx);
    click(Box::leak(format!("thread-{id}").into_boxed_str()), cx);
    draw(&root, cx);
    click("reopen-thread", cx);
    draw(&root, cx);
    let reopened = cx.update(|_, cx| view.read(cx).threads.read(cx).records[0].clone());
    assert!(reopened.closed_at.is_none());
    assert_eq!(reopened.state, ResolverState::Stopped);
    assert_eq!(executor.resolver_runs.lock().unwrap().len(), 1);
}

#[gpui::test]
fn a_questions_thread_can_close_without_starting_a_resolver(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![selected_mr("Explain this change.")],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        quiet_config(""),
    );
    let cx = &mut visual;
    click("mr-1-2", cx);
    finish(&view, &root, cx);
    click("ask-claude", cx);
    finish(&view, &root, cx);
    pump_threads(&view, &root, cx);
    click("close-thread", cx);
    pump_threads(&view, &root, cx);
    assert!(cx.update(|_, cx| {
        view.read(cx).threads.read(cx).records[0]
            .closed_at
            .is_some()
    }));
    assert!(executor.resolver_runs.lock().unwrap().is_empty());
    assert_eq!(
        cx.update(|_, cx| view.read(cx).threads.read(cx).launched.len()),
        1
    );
}

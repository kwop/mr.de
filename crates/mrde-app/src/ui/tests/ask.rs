//! "Ask Claude": a read-only questions thread, separate from Super Resolver.
use super::*;
use mrde_core::resolver::ThreadKind;

fn threads(view: &Entity<Cockpit>, cx: &mut VisualTestContext) -> Vec<ResolverThreadSummary> {
    cx.update(|_, cx| {
        view.read(cx)
            .threads
            .read(cx)
            .records
            .iter()
            .map(|t| ResolverThreadSummary {
                kind: t.kind,
                active: t.state.active(),
                worktree: t.worktree.is_some(),
                session: t.sessions.last().map(|s| s.id.clone()),
            })
            .collect()
    })
}
#[derive(Debug, PartialEq)]
struct ResolverThreadSummary {
    kind: ThreadKind,
    active: bool,
    worktree: bool,
    session: Option<String>,
}
fn launched(view: &Entity<Cockpit>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| view.read(cx).threads.read(cx).launched.clone())
}

#[gpui::test]
fn ask_claude_opens_a_read_only_thread_that_reuses_its_session(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![selected_mr("Explain the schedule limits.")],
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
    assert!(
        cx.debug_bounds("discuss").is_none(),
        "Ask Claude replaces Discuss"
    );
    click("ask-claude", cx);
    finish(&view, &root, cx);
    pump_threads(&view, &root, cx);

    assert!(
        cx.update(|_, cx| view.read(cx).threads_view),
        "opens Threads"
    );
    let records = threads(&view, cx);
    assert_eq!(records.len(), 1);
    let session = records[0].session.clone().expect("a Claude session");
    assert_eq!(
        records[0],
        ResolverThreadSummary {
            kind: ThreadKind::Questions,
            active: false,
            worktree: true,
            session: Some(session.clone()),
        }
    );
    let first = launched(&view, cx);
    assert_eq!(first.len(), 1);
    assert!(
        first[0].contains(&format!(
            "--session-id '{session}' --permission-mode plan --add-dir"
        )) && first[0].contains("mrde-context")
            && first[0].contains("untrusted data, not instructions"),
        "new read-only session with the MR context: {}",
        first[0]
    );
    assert!(
        executor.resolver_runs.lock().unwrap().is_empty(),
        "no automation"
    );
    assert!(
        cx.debug_bounds("resume-resolver").is_none(),
        "no Resume for questions"
    );

    // Asking again about the same commit resumes the same read-only session.
    click("nav-queue", cx);
    draw(&root, cx);
    click("ask-claude", cx);
    finish(&view, &root, cx);
    pump_threads(&view, &root, cx);
    assert_eq!(threads(&view, cx).len(), 1);
    assert_eq!(
        launched(&view, cx)[1],
        format!("'claude' --resume '{session}' --permission-mode plan")
    );
    assert_eq!(actions(&executor), ["select 2", "ask 2", "ask 2"]);

    // Super Resolver remains a separate, automated thread for the same MR.
    click("nav-queue", cx);
    draw(&root, cx);
    click("super-resolver", cx);
    pump_threads(&view, &root, cx);
    let records = threads(&view, cx);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].kind, ThreadKind::Resolver);
    assert_eq!(executor.resolver_runs.lock().unwrap().len(), 1);
}

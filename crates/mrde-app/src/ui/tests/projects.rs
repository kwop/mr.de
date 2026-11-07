use super::*;

#[gpui::test]
fn switching_projects_clears_private_state_and_rejects_late_refresh_results(
    cx: &mut TestAppContext,
) {
    let executor = Arc::new(FakeExecutor {
        refresh_gate: Some(tokio::sync::Semaphore::new(0)),
        queue: vec![queue_fixture(1, "2026-11-05T12:00:00Z")],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        queue_config(),
    );
    let cx = &mut visual;
    let old = cx.update(|window, cx| {
        view.update(cx, |state, cx| {
            state.projects.push(Config {
                id: "second".into(),
                name: "Second".into(),
                ..queue_config()
            });
            state
                .token
                .update(cx, |s, cx| s.set_value("private-token", window, cx));
            state.filter.labels.push("private-label".into());
            state.confirm = true;
            state.start(Action::Refresh, "Refreshing", cx);
            state.generation
        })
    });
    draw(&root, cx);
    click("project-switcher", cx);
    draw(&root, cx);
    click("project-second", cx);
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(5));
        cx.run_until_parked();
        if cx.update(|_, cx| !view.read(cx).initializing) {
            break;
        }
    }
    cx.update(|_, cx| {
        view.update(cx, |state, _| {
            assert_eq!(state.config.id, "second");
            assert!(state.queue.is_empty());
            assert!(state.filter.labels.is_empty());
            assert!(!state.confirm);
            assert!(
                state
                    .project_threads
                    .contains_key(&queue_config().namespace())
            );
            state
                .sender
                .send((
                    old,
                    Ok(Outcome::Queue(vec![queue_fixture(
                        99,
                        "2026-11-05T12:00:00Z",
                    )])),
                ))
                .unwrap();
        })
    });
    cx.update(|_, cx| view.update(cx, |state, cx| state.poll(cx)));
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(state.queue.is_empty());
        assert!(state.token.read(cx).value().is_empty());
    });
}

#[gpui::test]
fn new_project_form_saves_independently_and_preserves_the_existing_profile(
    cx: &mut TestAppContext,
) {
    let service = Arc::new(ControlledSettings::new(false));
    let (view, root, mut visual) = setup(cx, service.clone());
    let cx = &mut visual;
    draw(&root, cx);
    click("project-switcher", cx);
    draw(&root, cx);
    click("new-project", cx);
    draw(&root, cx);
    cx.update(|window, cx| {
        view.update(cx, |state, cx| {
            assert!(state.editing_project.is_some());
            assert!(state.token.read(cx).value().is_empty());
            state
                .project_name
                .update(cx, |s, cx| s.set_value("Community", window, cx));
            state.url.update(cx, |s, cx| {
                s.set_value("https://gitlab.example.com", window, cx)
            });
            state
                .token
                .update(cx, |s, cx| s.set_value("second-token", window, cx));
        })
    });
    click("save-settings", cx);
    service.gate.add_permits(1);
    finish(&view, &root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert_eq!(state.projects.len(), 2);
        assert_eq!(state.config.name, "Community");
        assert_ne!(state.config.id, "default");
        assert!(state.projects.iter().any(|p| p.id == "default"));
        assert_eq!(state.token.read(cx).value().as_str(), "second-token");
        assert!(!state.feedback_error);
    });
}

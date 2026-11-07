use super::*;
use mrde_core::language::Language;

#[gpui::test]
fn language_controls_save_independently_and_preserve_secrets(cx: &mut TestAppContext) {
    let service = Arc::new(ControlledSettings::new(false));
    let (view, root, mut visual) = setup(cx, service.clone());
    let cx = &mut visual;
    cx.update(|window, cx| {
        view.update(cx, |state, cx| {
            state.token.update(cx, |input, cx| {
                input.set_value("synthetic-token", window, cx)
            });
        })
    });
    draw(&root, cx);
    click("nav-languages", cx);
    draw(&root, cx);
    click("language-interface-fr", cx);
    click("save-languages", cx);
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(5));
        cx.run_until_parked();
        if cx.update(|_, cx| !view.read(cx).language_saving) {
            break;
        }
    }
    draw(&root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert_eq!(state.config.languages.interface, Language::French);
        assert_eq!(state.config.languages.information, Language::English);
        assert_eq!(state.token.read(cx).value().as_str(), "synthetic-token");
        assert_eq!(state.language_feedback, "Languages saved");
        assert!(!state.feedback_error);
    });
    assert_eq!(service.languages.lock().unwrap().len(), 1);
    assert_eq!(service.saves.load(Ordering::SeqCst), 0);
    assert_eq!(service.tests.load(Ordering::SeqCst), 0);
}

#[gpui::test]
fn failed_language_save_keeps_the_draft_and_previous_preferences(cx: &mut TestAppContext) {
    let service = Arc::new(ControlledSettings::new(true));
    let (view, root, mut visual) = setup(cx, service);
    let cx = &mut visual;
    draw(&root, cx);
    click("nav-languages", cx);
    draw(&root, cx);
    click("language-information-fr", cx);
    click("save-languages", cx);
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(5));
        cx.run_until_parked();
        if cx.update(|_, cx| !view.read(cx).language_saving) {
            break;
        }
    }
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert_eq!(state.config.languages.information, Language::English);
        assert_eq!(state.language_draft.information, Language::French);
        assert!(state.language_feedback.contains("Cannot save languages"));
    });
}

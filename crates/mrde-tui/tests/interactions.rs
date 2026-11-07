use mrde_core::{language::Language, models::MrDetail};
use mrde_service::{
    backend::{Action, Outcome},
    demo,
};
use mrde_tui::{
    app::{App, Effect, Screen, Tab},
    ui::View,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};

fn key(app: &mut App, code: KeyCode) -> Vec<Effect> {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn token(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Run { token, .. } => Some(*token),
            _ => None,
        })
        .unwrap()
}
fn detail(iid: u64) -> Box<MrDetail> {
    Box::new(
        demo::queue()
            .iter()
            .find(|mr| mr.id.iid == iid)
            .and_then(|mr| demo::detail(&mr.id))
            .unwrap(),
    )
}
fn rendered(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    View::default().draw_for_test(&mut terminal, app);
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
trait Draw {
    fn draw_for_test(&mut self, terminal: &mut Terminal<TestBackend>, app: &mut App);
}
impl Draw for View {
    fn draw_for_test(&mut self, terminal: &mut Terminal<TestBackend>, app: &mut App) {
        terminal.draw(|frame| self.draw(frame, app)).unwrap();
    }
}

#[test]
fn approval_requires_confirmation_and_uses_the_displayed_commit() {
    let mut app = App::demo();
    app.screen = Screen::Reader;
    app.detail = Some(detail(231));
    assert!(key(&mut app, KeyCode::Char('a')).is_empty());
    assert!(app.confirmation.is_some());
    assert!(key(&mut app, KeyCode::Char('n')).is_empty());
    assert!(app.confirmation.is_none());
    key(&mut app, KeyCode::Char('a'));
    let effects = key(&mut app, KeyCode::Char('y'));
    assert!(
        matches!(&effects[0], Effect::Run { action: Action::Approve(d), .. } if d.mr.sha==app.detail.as_ref().unwrap().mr.sha)
    );
}
#[test]
fn pending_refresh_does_not_block_navigation_and_late_details_are_ignored() {
    let mut app = App::demo();
    let refresh = app.request(Action::Refresh);
    let refresh_token = token(&refresh);
    key(&mut app, KeyCode::Char(','));
    assert_eq!(app.screen, Screen::Settings);
    key(&mut app, KeyCode::Char('i'));
    let first = key(&mut app, KeyCode::Enter);
    let first_token = token(&first);
    key(&mut app, KeyCode::Char('i'));
    key(&mut app, KeyCode::Down);
    let second = key(&mut app, KeyCode::Enter);
    let second_token = token(&second);
    app.complete(first_token, Ok(Outcome::Detail(detail(231), None)));
    assert!(app.detail.is_none());
    app.complete(second_token, Ok(Outcome::Detail(detail(244), None)));
    assert_eq!(app.detail.as_ref().unwrap().mr.id.iid, 244);
    app.complete(refresh_token, Ok(Outcome::Queue(demo::queue())));
    assert_eq!(app.screen, Screen::Reader);
}
#[test]
fn a_risk_result_for_a_previous_selection_cannot_replace_the_current_mr() {
    let mut app = App::demo();
    app.screen = Screen::Reader;
    app.detail = Some(detail(231));
    let effects = key(&mut app, KeyCode::Char('r'));
    let id = token(&effects);
    app.detail = Some(detail(244));
    app.complete(
        id,
        Ok(Outcome::Risk(
            detail(231).mr.id.clone(),
            Box::new(demo::risk(&detail(231))),
        )),
    );
    assert!(app.risk.is_none());
    assert!(!app.busy());
}
#[test]
fn renderer_supports_both_languages_and_missing_measurements() {
    for language in Language::ALL {
        for (width, height) in [(80, 24), (100, 32), (140, 44)] {
            let mut app = App::demo();
            app.config.languages.interface = language;
            for screen in [
                "inbox",
                "description",
                "files",
                "radar",
                "review",
                "history",
                "settings",
                "projects",
                "worktrees",
            ] {
                mrde_tui::snapshot::scenario(&mut app, screen).unwrap();
                let text = rendered(&mut app, width, height);
                assert!(text.contains("mr.de"), "{screen} {width}x{height}");
                if screen == "radar" && height >= 32 {
                    assert!(text.contains("n/a"));
                }
                if screen == "settings" {
                    assert!(text.contains(if language == Language::French {
                        "Langues"
                    } else {
                        "Languages"
                    }));
                }
            }
        }
    }
}
#[test]
fn end_scroll_reveals_final_content_and_resizes_stay_in_bounds() {
    let mut app = App::demo();
    app.screen = Screen::Reader;
    app.tab = Tab::Description;
    let mut detail = detail(231);
    detail.mr.description = format!(
        "{}\nLAST_VISIBLE_SENTINEL",
        "A paragraph that wraps onto several lines in a narrow terminal.\n\n".repeat(150)
    );
    detail.issues.clear();
    app.detail = Some(detail);
    key(&mut app, KeyCode::End);
    let text = rendered(&mut app, 80, 24);
    assert!(text.contains("LAST_VISIBLE_SENTINEL"));
    key(&mut app, KeyCode::Home);
    let text = rendered(&mut app, 100, 32);
    assert!(!text.contains("LAST_VISIBLE_SENTINEL"));
}
#[test]
fn search_matches_scoped_labels_and_preserves_source_titles() {
    let mut app = App::demo();
    let expected = app.queue[0].title.clone();
    let label = app.queue[0].all_labels().next().unwrap().name.clone();
    key(&mut app, KeyCode::Char('/'));
    for c in label.chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    assert!(app.filtered().iter().any(|mr| mr.title == expected));
    app.config.languages.interface = Language::French;
    assert!(app.filtered().iter().any(|mr| mr.title == expected));
}

#[test]
fn worker_runtime_keeps_input_available_while_the_forge_is_pending() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct SlowForge {
        started: AtomicBool,
        release: tokio::sync::Semaphore,
    }
    #[async_trait::async_trait]
    impl mrde_service::backend::Executor for SlowForge {
        async fn execute(
            &self,
            _: mrde_core::config::Config,
            _: Action,
        ) -> anyhow::Result<Outcome> {
            self.started.store(true, Ordering::SeqCst);
            self.release.acquire().await?.forget();
            Ok(Outcome::Queue(demo::queue()))
        }
    }
    let forge = Arc::new(SlowForge {
        started: AtomicBool::new(false),
        release: tokio::sync::Semaphore::new(0),
    });
    let mut driver = mrde_tui::runtime::Driver::new(
        forge.clone(),
        Arc::new(mrde_service::settings::PreviewSettings),
    )
    .unwrap();
    let mut app = App::demo();
    driver.submit(app.request(Action::Refresh));
    for _ in 0..100 {
        if forge.started.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(forge.started.load(Ordering::SeqCst));
    key(&mut app, KeyCode::Char(','));
    key(&mut app, KeyCode::Char('1'));
    assert_eq!(app.screen, Screen::Settings);
    assert_eq!(app.language_draft.interface, Language::French);
    assert!(!driver.poll(&mut app).0);
    assert!(rendered(&mut app, 100, 32).contains("Languages"));
    forge.release.add_permits(1);
    let mut answered = false;
    for _ in 0..100 {
        if driver.poll(&mut app).0 {
            answered = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(answered);
    assert_eq!(app.screen, Screen::Settings);
}

#[test]
fn confirmation_keeps_commit_and_buttons_visible_with_long_scrolled_context() {
    let mut app = App::demo();
    app.screen = Screen::Reader;
    let mut detail = detail(231);
    detail.mr.title = "A long title ".repeat(100);
    let sha = detail.mr.sha.clone();
    app.detail = Some(detail);
    key(&mut app, KeyCode::Char('a'));
    let text = rendered(&mut app, 80, 24);
    assert!(text.contains(&sha));
    assert!(text.contains("y Confirm"));
    key(&mut app, KeyCode::End);
    let text = rendered(&mut app, 80, 24);
    assert!(text.contains(&sha));
    assert!(text.contains("y Confirm"));
    assert!(app.confirmation.is_some());
}

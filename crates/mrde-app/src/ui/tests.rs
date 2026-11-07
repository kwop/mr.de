use super::{Cockpit, DetailTab, SettingsOperation};
use crate::backend::{Action, Executor, Outcome};
use crate::settings::{self, ConnectionSettings, Draft};
use async_trait::async_trait;
use gpui::{AppContext, Entity, Modifiers, TestAppContext, VisualTestContext, point, px, size};
use gpui_component::Root;
use mrde_core::config::{Config, MrRole, QueueSort};
use mrde_core::models::{Label, MergeRequest, MrDetail, MrId, ReviewLevel};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{sync::Arc, time::Duration};

struct ControlledSettings {
    gate: tokio::sync::Semaphore,
    fail: bool,
    tests: AtomicUsize,
    saves: AtomicUsize,
    preferences: std::sync::Mutex<Vec<(Vec<MrRole>, QueueSort)>>,
}
impl ControlledSettings {
    fn new(fail: bool) -> Self {
        Self {
            gate: tokio::sync::Semaphore::new(0),
            fail,
            tests: 0.into(),
            saves: 0.into(),
            preferences: Default::default(),
        }
    }
}
#[async_trait]
impl ConnectionSettings for ControlledSettings {
    async fn activate_project(&self, id: String) -> anyhow::Result<Config> {
        if self.fail {
            anyhow::bail!("Cannot activate project");
        }
        Ok(Config {
            id,
            name: "Second".into(),
            ..queue_config()
        })
    }

    async fn save_queue_preferences(
        &self,
        _project: Config,
        roles: Vec<MrRole>,
        sort: QueueSort,
    ) -> anyhow::Result<Config> {
        if self.fail {
            anyhow::bail!("Cannot write queue preferences");
        }
        self.preferences.lock().unwrap().push((roles.clone(), sort));
        Ok(Config {
            gitlab_url: "https://gitlab.example.com".into(),
            mr_roles: roles,
            mr_sort: sort,
            refresh_seconds: u64::MAX / 2,
            ..Default::default()
        })
    }
    async fn test(&self, _: Draft, progress: settings::Progress) -> anyhow::Result<String> {
        self.tests.fetch_add(1, Ordering::SeqCst);
        progress("GET /api/v4/user".into());
        let _permit = self.gate.acquire().await?;
        if self.fail {
            anyhow::bail!("GitLab returned HTTP 401 for synthetic-token");
        }
        Ok("reviewer".into())
    }
    async fn save(&self, draft: Draft, progress: settings::Progress) -> anyhow::Result<Config> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        progress("Saving credentials to Keychain".into());
        let _permit = self.gate.acquire().await?;
        if self.fail {
            anyhow::bail!("Keychain access denied");
        }
        Ok(draft.config)
    }
}
/// Records actions and answers without GitLab, Git, Claude, or Keychain.
#[derive(Default)]
struct FakeExecutor {
    fail_images: bool,
    refresh_gate: Option<tokio::sync::Semaphore>,
    refresh_started: AtomicUsize,
    operation_gate: Option<tokio::sync::Semaphore>,
    operation_started: AtomicUsize,
    actions: std::sync::Mutex<Vec<String>>,
    queue: Vec<MergeRequest>,
    refreshed_roles: std::sync::Mutex<Vec<Vec<MrRole>>>,
    worktrees: Vec<mrde_git::WorktreeEntry>,
    resolver_runs: std::sync::Mutex<Vec<mrde_core::resolver::ResolverThread>>,
    resolver_updates: std::sync::Mutex<Vec<mrde_resolver::Emit>>,
    resolver_gate: Option<tokio::sync::Semaphore>,
}
#[async_trait]
impl Executor for FakeExecutor {
    async fn resolve(
        &self,
        _: Config,
        mut thread: mrde_core::resolver::ResolverThread,
        emit: mrde_resolver::Emit,
    ) -> anyhow::Result<()> {
        use mrde_core::resolver::*;
        thread.attempts = 1;
        thread.state = ResolverState::FixingPipeline;
        thread.sessions.push(ClaudeSession {
            id: mrde_resolver::Uuid::new_v4().to_string(),
            purpose: "Repair".into(),
            started_at: chrono::Utc::now(),
            finished_at: None,
            output: "Inspecting failed tests".into(),
        });
        thread.event("Repair started");
        self.resolver_runs.lock().unwrap().push(thread.clone());
        self.resolver_updates.lock().unwrap().push(emit.clone());
        emit(mrde_resolver::Update::Thread(Box::new(thread.clone())));
        if let Some(gate) = &self.resolver_gate {
            gate.acquire().await?.forget();
        }
        thread.block("Should an empty summary be skipped?");
        emit(mrde_resolver::Update::Thread(Box::new(thread)));
        Ok(())
    }
    async fn download_image(&self, _: Config, url: String) -> anyhow::Result<Vec<u8>> {
        self.actions.lock().unwrap().push(format!("image {url}"));
        if self.fail_images {
            anyhow::bail!("Image download failed");
        }
        if url.ends_with("/portrait.svg") {
            return Ok(crate::demo::portrait_image());
        }
        Ok(br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#4dc179"/></svg>"##.to_vec())
    }
    async fn execute(&self, config: Config, action: Action) -> anyhow::Result<Outcome> {
        if matches!(action, Action::Refresh) {
            self.actions.lock().unwrap().push("refresh".into());
            self.refreshed_roles.lock().unwrap().push(config.mr_roles);
            self.refresh_started.fetch_add(1, Ordering::SeqCst);
            if let Some(gate) = &self.refresh_gate {
                gate.acquire().await?.forget();
            }
            return Ok(Outcome::Queue(self.queue.clone()));
        }
        if !matches!(action, Action::Select(_) | Action::Worktrees)
            && let Some(gate) = &self.operation_gate
        {
            self.operation_started.fetch_add(1, Ordering::SeqCst);
            gate.acquire().await?.forget();
        }
        let mut actions = self.actions.lock().unwrap();
        match action {
            Action::Select(id) => {
                actions.push(format!("select {}", id.iid));
                let mr = self.queue.iter().find(|mr| mr.id == id).unwrap().clone();
                Ok(Outcome::Detail(
                    Box::new(MrDetail {
                        mr,
                        files: vec![],
                        issues: vec![],
                        ssh_url: String::new(),
                    }),
                    None,
                ))
            }
            Action::Approve(detail) => {
                actions.push(format!("approve {}", detail.mr.sha));
                Ok(Outcome::Approved(detail.mr.id.clone(), Some(2)))
            }
            Action::Review(detail, level) => {
                actions.push(format!("review {} {}", detail.mr.id.iid, level.label()));
                Ok(Outcome::Review(
                    level,
                    mrde_core::models::Review {
                        summary: "Looks consistent.".into(),
                        intention: "Fix the queue.".into(),
                        risks: vec![],
                        priority_files: vec![],
                        confidence: 0.9,
                    },
                ))
            }
            Action::Risk(detail) | Action::ApproveRiskCommand(detail, _) => {
                actions.push("risk".into());
                Ok(Outcome::Risk(
                    detail.mr.id.clone(),
                    Box::new(crate::demo::risk(&detail)),
                ))
            }
            Action::RiskBaseline(detail) => {
                actions.push("baseline".into());
                Ok(Outcome::RiskBaseline(
                    detail.mr.id.clone(),
                    crate::demo::baseline(),
                ))
            }
            Action::RiskFile(detail, path) => {
                actions.push("risk-file".into());
                Ok(Outcome::RiskFile(
                    detail.mr.id.clone(),
                    detail.mr.sha.clone(),
                    crate::demo::file_context(&path),
                ))
            }
            Action::AskRisk(detail, _, _) => {
                actions.push("ask-risk".into());
                let directory = std::env::temp_dir();
                Ok(Outcome::Ask(Box::new(crate::backend::AskReady::new(
                    &detail,
                    directory.clone(),
                    directory.clone(),
                    &directory.join("context.md"),
                ))))
            }
            Action::Statistics(detail) => {
                actions.push("statistics".into());
                let stat = |path: &str, added| mrde_core::models::FileStat {
                    path: path.into(),
                    added,
                    removed: 1,
                    binary: false,
                };
                Ok(Outcome::Statistics(
                    detail.mr.id.clone(),
                    mrde_core::models::DiffStats {
                        sha: detail.mr.sha.clone(),
                        source: mrde_core::models::StatsSource::Git,
                        files: vec![stat("backend/api.go", 40), stat("frontend/app.ts", 12)],
                    },
                ))
            }
            Action::Compliance(detail) => {
                actions.push("compliance".into());
                Ok(Outcome::Compliance(
                    detail.mr.id.clone(),
                    detail.mr.sha.clone(),
                    serde_json::from_value(serde_json::json!({
                        "verdict":"partial","summary":"One criterion is missing.",
                        "spec_files":["epic-3/us-3.4.md"],
                        "findings":[{"requirement":"Edit a series","status":"missing","evidence":"No handler","spec_file":"epic-3/us-3.4.md"}],
                        "confidence":0.7
                    }))
                    .unwrap(),
                ))
            }
            Action::Worktrees => {
                actions.push("worktrees".into());
                Ok(Outcome::Worktrees(self.worktrees.clone(), None))
            }
            Action::RemoveWorktree(entry) => {
                actions.push(format!("remove {}", entry.iid));
                let remaining = self
                    .worktrees
                    .iter()
                    .filter(|w| w.iid != entry.iid)
                    .cloned()
                    .collect();
                Ok(Outcome::Worktrees(
                    remaining,
                    Some("Worktree removed.".into()),
                ))
            }
            Action::Analyze(detail) => {
                actions.push(format!("analyze {}", detail.mr.id.iid));
                Ok(Outcome::Insights(
                    detail.mr.id.clone(),
                    Box::new(crate::demo::insights(&detail)),
                ))
            }
            Action::Tour(detail) => {
                actions.push(format!("tour {}", detail.mr.id.iid));
                Ok(Outcome::Tour(
                    detail.mr.id.clone(),
                    detail.mr.sha.clone(),
                    crate::demo::tour(),
                ))
            }
            Action::Visual(detail) => {
                actions.push(format!("visual {}", detail.mr.id.iid));
                Ok(Outcome::Visual(
                    detail.mr.id.clone(),
                    detail.mr.sha.clone(),
                    serde_json::from_value(serde_json::json!({"screens": [
                        {"name": "series-dialog", "before": null, "after": null, "diff": null, "changed": null}
                    ]}))
                    .unwrap(),
                ))
            }
            Action::Ask(detail, _) => {
                actions.push(format!("ask {}", detail.mr.id.iid));
                let directory = std::env::temp_dir();
                Ok(Outcome::Ask(Box::new(crate::backend::AskReady::new(
                    &detail,
                    directory.clone(),
                    directory.join("mrde-context"),
                    &directory.join("mrde-context/context.md"),
                ))))
            }
            Action::Open(reference) => {
                actions.push(format!("open {reference}"));
                let iid: u64 = reference.rsplit(['!', '/']).next().unwrap().parse()?;
                let Some(mr) = self.queue.iter().find(|mr| mr.id.iid == iid).cloned() else {
                    anyhow::bail!("Cannot open project");
                };
                Ok(Outcome::Detail(
                    Box::new(MrDetail {
                        mr,
                        files: vec![],
                        issues: vec![],
                        ssh_url: String::new(),
                    }),
                    None,
                ))
            }
            _ => anyhow::bail!("unexpected action"),
        }
    }
}
fn setup(
    cx: &mut TestAppContext,
    service: Arc<ControlledSettings>,
) -> (Entity<Cockpit>, Entity<Root>, VisualTestContext) {
    setup_with(
        cx,
        service,
        Arc::new(FakeExecutor::default()),
        Config::default(),
    )
}
fn setup_with(
    cx: &mut TestAppContext,
    service: Arc<ControlledSettings>,
    executor: Arc<FakeExecutor>,
    config: Config,
) -> (Entity<Cockpit>, Entity<Root>, VisualTestContext) {
    let queue = executor.queue.clone();
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::theme::init(cx);
    });
    let runtime = Arc::new(tokio::runtime::Runtime::new().unwrap());
    let mut view = None;
    let window = cx.update(|cx| {
        cx.open_window(gpui::WindowOptions::default(), |window, cx| {
            let cockpit = cx.new(|cx| {
                Cockpit::with_state(
                    runtime,
                    executor,
                    Some(service),
                    config,
                    queue,
                    "Ready".into(),
                    window,
                    cx,
                )
            });
            view = Some(cockpit.clone());
            cx.new(|cx| Root::new(cockpit, window, cx))
        })
        .unwrap()
    });
    let root = window.root(cx).unwrap();
    let mut visual = VisualTestContext::from_window(*window, cx);
    visual.simulate_resize(size(px(1280.), px(900.)));
    draw(&root, &mut visual);
    (view.unwrap(), root, visual)
}

fn draw(root: &Entity<Root>, cx: &mut VisualTestContext) {
    // Preparation and initial Markdown/diagram layout now complete asynchronously.
    for _ in 0..4 {
        cx.draw(point(px(0.), px(0.)), size(px(1280.), px(900.)), |_, _| {
            root.clone()
        });
        cx.run_until_parked();
    }
}
fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing control: {selector}"));
    assert!(
        bounds.origin.y >= px(0.) && bounds.bottom() <= px(900.),
        "Control is outside window: {selector} {bounds:?}"
    );
    cx.simulate_click(bounds.center(), Modifiers::none());
}
/// Reveal secondary tools in their tab, then scroll the reader before clicking.
fn click_action(selector: &'static str, root: &Entity<Root>, cx: &mut VisualTestContext) {
    let tab = match selector {
        "compliance-check" => Some("tab-checks"),
        "statistics" => Some("tab-statistics"),
        _ => None,
    };
    if let Some(tab) = tab {
        click(tab, cx);
        draw(root, cx);
    }
    let panel = cx.debug_bounds("mr-detail").unwrap();
    let control = cx.debug_bounds(selector).unwrap();
    if control.bottom() > panel.bottom() || control.origin.y < panel.origin.y {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: panel.center(),
            delta: gpui::ScrollDelta::Pixels(point(
                px(0.),
                panel.origin.y + px(16.) - control.origin.y,
            )),
            touch_phase: gpui::TouchPhase::Moved,
            ..Default::default()
        });
        draw(root, cx);
    }
    let control = cx.debug_bounds(selector).unwrap();
    assert!(
        control.origin.y >= panel.origin.y && control.bottom() <= panel.bottom(),
        "Action must be visible inside its scrolling panel"
    );
    click(selector, cx);
}
fn fill(view: &Entity<Cockpit>, root: &Entity<Root>, cx: &mut VisualTestContext) {
    click("gitlab-url", cx);
    cx.simulate_input("https://gitlab.example.com");
    draw(root, cx);
    click("gitlab-token", cx);
    cx.simulate_input("synthetic-token");
    cx.update(|_, cx| view.read(cx).token.read(cx).value().to_string())
        .eq("synthetic-token")
        .then_some(())
        .expect("Keyboard input did not reach token field");
    draw(root, cx);
}
fn finish(view: &Entity<Cockpit>, root: &Entity<Root>, cx: &mut VisualTestContext) {
    for _ in 0..200 {
        std::thread::sleep(Duration::from_millis(5));
        cx.run_until_parked();
        if cx.update(|_, cx| view.read(cx).job.is_none() && view.read(cx).navigation_job.is_none())
        {
            draw(root, cx);
            return;
        }
    }
    panic!("UI never received the background result");
}

#[gpui::test]
fn click_test_shows_busy_retains_token_and_never_saves(cx: &mut TestAppContext) {
    let service = Arc::new(ControlledSettings::new(false));
    let (view, root, mut visual) = setup(cx, service.clone());
    let cx = &mut visual;
    fill(&view, &root, cx);
    click("test-connection", cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(state.settings_operation == Some(SettingsOperation::Test));
        assert!(state.job.is_some());
        assert_eq!(state.token.read(cx).value(), "synthetic-token");
    });
    draw(&root, cx);
    click("test-connection", cx); // disabled while running; must not submit twice
    service.gate.add_permits(1);
    finish(&view, &root, cx);
    assert_eq!(service.tests.load(Ordering::SeqCst), 1);
    assert_eq!(service.saves.load(Ordering::SeqCst), 0);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(state.feedback.contains("Connected as reviewer"));
        assert_eq!(state.token.read(cx).value(), "synthetic-token");
        assert!(state.settings_operation.is_none());
    });
    click("toggle-debug", cx);
    draw(&root, cx);
    assert!(cx.debug_bounds("debug-panel").is_some());
    click("toggle-debug", cx);
    assert!(!cx.update(|_, cx| view.read(cx).logs_open));
}
#[gpui::test]
fn failed_test_is_visible_redacted_and_retryable(cx: &mut TestAppContext) {
    let service = Arc::new(ControlledSettings::new(true));
    let (view, root, mut visual) = setup(cx, service.clone());
    let cx = &mut visual;
    fill(&view, &root, cx);
    click("test-connection", cx);
    service.gate.add_permits(1);
    finish(&view, &root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(state.feedback_error);
        assert!(state.feedback.contains("401"));
        assert!(state.logs_open);
        assert!(
            !state
                .logs
                .iter()
                .any(|line| line.contains("synthetic-token"))
        );
        assert_eq!(state.token.read(cx).value(), "synthetic-token");
        assert!(state.settings_operation.is_none());
    });
    assert_eq!(service.saves.load(Ordering::SeqCst), 0);
}
#[gpui::test]
fn save_click_shows_confirmation_without_testing_or_leaving_settings(cx: &mut TestAppContext) {
    let service = Arc::new(ControlledSettings::new(false));
    let (view, root, mut visual) = setup(cx, service.clone());
    let cx = &mut visual;
    fill(&view, &root, cx);
    click("save-settings", cx);
    assert!(cx.update(|_, cx| view.read(cx).settings_operation == Some(SettingsOperation::Save)));
    service.gate.add_permits(1);
    finish(&view, &root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(state.feedback.starts_with("Saved."));
        assert!(state.settings);
        assert_eq!(state.token.read(cx).value(), "synthetic-token");
    });
    assert_eq!(service.tests.load(Ordering::SeqCst), 0);
    assert_eq!(service.saves.load(Ordering::SeqCst), 1);
}
fn queue_fixture(iid: u64, created_at: &str) -> MergeRequest {
    serde_json::from_value(serde_json::json!({
        "id":{"project":1,"iid":iid}, "title":"Handle empty daily summaries", "description":"A short description.",
        "project":"team/orbit", "author":"mara", "web_url":"https://example.invalid/mr/231",
        "sha":"a".repeat(40), "base_sha":"b".repeat(40), "created_at":created_at,
        "labels":["P1", "backend", "needs review"], "pipeline":"success", "approvals":1, "changed_lines":42
    })).unwrap()
}
fn queue_config() -> Config {
    Config {
        gitlab_url: "https://gitlab.example.com".into(),
        refresh_seconds: u64::MAX / 2,
        ..Default::default()
    }
}
/// Keep GitLab pending until the test explicitly releases it, independent of machine speed.
fn blocked_refresh_executor(count: u64) -> Arc<FakeExecutor> {
    Arc::new(FakeExecutor {
        refresh_gate: Some(tokio::sync::Semaphore::new(0)),
        queue: (1..=count)
            .map(|iid| queue_fixture(iid, "2026-11-05T12:00:00Z"))
            .collect(),
        ..Default::default()
    })
}
fn begin_refresh(
    view: &Entity<Cockpit>,
    root: &Entity<Root>,
    executor: &FakeExecutor,
    automatic: bool,
    cx: &mut VisualTestContext,
) -> u64 {
    let started = executor.refresh_started.load(Ordering::SeqCst);
    if automatic {
        cx.update(|_, cx| view.update(cx, |state, cx| state.refresh(true, cx)));
    } else {
        click("refresh-queue", cx);
    }
    for _ in 0..200 {
        if executor.refresh_started.load(Ordering::SeqCst) > started {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
        cx.run_until_parked();
    }
    assert_eq!(executor.refresh_started.load(Ordering::SeqCst), started + 1);
    draw(root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(state.refreshing && state.job.is_some());
        assert!(!state.is_busy());
        assert_eq!(state.automatic, automatic);
        state.generation
    })
}
fn deliver_stale_refresh(view: &Entity<Cockpit>, generation: u64, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            assert_ne!(generation, state.generation);
            state
                .sender
                .send((generation, Ok(Outcome::Queue(vec![]))))
                .unwrap();
            state
                .sender
                .send((generation, Err(anyhow::anyhow!("Stale refresh failed"))))
                .unwrap();
            state
                .progress_sender
                .send((generation, "Stale progress".into()))
                .unwrap();
            state.poll(cx);
            assert!(!state.logs.iter().any(|line| line.contains("Stale")));
        })
    });
}

#[gpui::test]
fn pending_refresh_keeps_navigation_pagination_and_settings_input_responsive(
    cx: &mut TestAppContext,
) {
    for automatic in [false, true] {
        let executor = blocked_refresh_executor(30);
        let (view, root, mut visual) = setup_with(
            cx,
            Arc::new(ControlledSettings::new(false)),
            executor.clone(),
            queue_config(),
        );
        let cx = &mut visual;
        let generation = begin_refresh(&view, &root, &executor, automatic, cx);
        click("refresh-queue", cx); // Only the refresh button is disabled.
        click("next-page", cx);
        draw(&root, cx);
        assert_eq!(cx.update(|_, cx| view.read(cx).queue_page), 1);
        click("nav-settings", cx);
        draw(&root, cx);
        assert!(cx.update(|_, cx| view.read(cx).settings));
        click("gitlab-token", cx);
        cx.simulate_input("synthetic-token");
        draw(&root, cx);
        click("nav-queue", cx);
        draw(&root, cx);
        assert!(!cx.update(|_, cx| view.read(cx).settings));
        click("nav-settings", cx);
        draw(&root, cx);
        cx.update(|_, cx| {
            let state = view.read(cx);
            assert!(state.refreshing);
            assert_eq!(state.generation, generation);
            assert_eq!(state.token.read(cx).value(), "synthetic-token");
        });
        executor.refresh_gate.as_ref().unwrap().add_permits(1);
        finish(&view, &root, cx);
        cx.update(|_, cx| {
            let state = view.read(cx);
            assert!(state.settings);
            assert!(!state.refreshing);
            assert_eq!(state.queue.len(), 30);
            assert_eq!(state.token.read(cx).value(), "synthetic-token");
        });
        assert_eq!(executor.refresh_started.load(Ordering::SeqCst), 1);
    }
}

fn settings_preempts_refresh(operation: SettingsOperation, cx: &mut TestAppContext) {
    let executor = blocked_refresh_executor(1);
    let service = Arc::new(ControlledSettings::new(false));
    let (view, root, mut visual) =
        setup_with(cx, service.clone(), executor.clone(), queue_config());
    let cx = &mut visual;
    let generation = begin_refresh(&view, &root, &executor, true, cx);
    click("nav-settings", cx);
    draw(&root, cx);
    click("gitlab-url", cx);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("https://new-gitlab.example.com");
    draw(&root, cx);
    click("gitlab-token", cx);
    cx.simulate_input("synthetic-token");
    draw(&root, cx);
    click(
        if operation == SettingsOperation::Test {
            "test-connection"
        } else {
            "save-settings"
        },
        cx,
    );
    deliver_stale_refresh(&view, generation, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(!state.refreshing);
        assert!(!state.automatic);
        assert!(state.is_busy());
        assert!(state.settings_operation == Some(operation));
        assert_eq!(state.queue.len(), 1);
        assert_eq!(state.url.read(cx).value(), "https://new-gitlab.example.com");
    });
    service.gate.add_permits(1);
    finish(&view, &root, cx);
    deliver_stale_refresh(&view, generation, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(state.settings && !state.feedback_error);
        assert!(!state.refreshing && !state.is_busy());
        assert_eq!(state.token.read(cx).value(), "synthetic-token");
        if operation == SettingsOperation::Test {
            assert!(state.feedback.contains("Connected as reviewer"));
            assert_eq!(state.config.gitlab_url, "https://gitlab.example.com");
            assert_eq!(state.queue.len(), 1);
        } else {
            assert!(state.feedback.starts_with("Saved."));
            assert_eq!(state.config.gitlab_url, "https://new-gitlab.example.com");
            assert!(state.queue.is_empty());
        }
    });
    assert_eq!(
        service.tests.load(Ordering::SeqCst),
        usize::from(operation == SettingsOperation::Test)
    );
    assert_eq!(
        service.saves.load(Ordering::SeqCst),
        usize::from(operation == SettingsOperation::Save)
    );
}
#[gpui::test]
fn connection_test_preempts_refresh_and_ignores_its_late_responses(cx: &mut TestAppContext) {
    settings_preempts_refresh(SettingsOperation::Test, cx);
}
#[gpui::test]
fn settings_save_preempts_refresh_and_ignores_old_configuration_results(cx: &mut TestAppContext) {
    settings_preempts_refresh(SettingsOperation::Save, cx);
}
#[gpui::test]
fn selecting_and_confirming_mr_preempt_refresh_without_losing_the_reviewed_sha(
    cx: &mut TestAppContext,
) {
    let executor = blocked_refresh_executor(1);
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        queue_config(),
    );
    let cx = &mut visual;
    let generation = begin_refresh(&view, &root, &executor, false, cx);
    click("mr-1-1", cx);
    finish(&view, &root, cx);
    deliver_stale_refresh(&view, generation, cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).detail.as_ref().unwrap().mr.id.iid),
        1
    );
    let generation = begin_refresh(&view, &root, &executor, false, cx);
    click("approve", cx);
    draw(&root, cx);
    deliver_stale_refresh(&view, generation, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(state.confirm);
        assert!(!state.refreshing && state.job.is_none());
        assert_eq!(state.detail.as_ref().unwrap().mr.sha, "a".repeat(40));
        assert_eq!(state.queue.len(), 1);
    });
    click("cancel-approval", cx);
    assert!(!cx.update(|_, cx| view.read(cx).confirm));
    assert_eq!(
        *executor.actions.lock().unwrap(),
        ["refresh", "select 1", "refresh"]
    );
}
#[gpui::test]
fn sorting_during_refresh_discards_the_old_queue_order(cx: &mut TestAppContext) {
    let executor = blocked_refresh_executor(1);
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        queue_config(),
    );
    let cx = &mut visual;
    let generation = begin_refresh(&view, &root, &executor, false, cx);
    click("sort-newest", cx);
    finish(&view, &root, cx);
    deliver_stale_refresh(&view, generation, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert_eq!(state.config.mr_sort, QueueSort::Newest);
        assert!(!state.refreshing);
        assert_eq!(state.queue.len(), 1);
    });
}

#[gpui::test]
fn inbox_filters_save_without_connection_settings_and_sort_without_refetching(
    cx: &mut TestAppContext,
) {
    let service = Arc::new(ControlledSettings::new(false));
    let executor = Arc::new(FakeExecutor {
        queue: vec![
            queue_fixture(1, "2026-11-05T12:00:00Z"),
            queue_fixture(2, "2026-11-06T12:00:00Z"),
        ],
        ..Default::default()
    });
    let (view, root, mut visual) =
        setup_with(cx, service.clone(), executor.clone(), queue_config());
    let cx = &mut visual;
    let navigation = cx.debug_bounds("navigation").unwrap();
    let toolbar = cx.debug_bounds("queue-toolbar").unwrap();
    let queue = cx.debug_bounds("mr-queue").unwrap();
    assert!(toolbar.origin.x >= navigation.right());
    assert!(toolbar.bottom() <= queue.origin.y);
    assert!(
        cx.debug_bounds("mr-actions").is_none(),
        "Actions need a selected MR"
    );
    assert!(cx.debug_bounds("mr-detail").is_none());
    assert!(cx.debug_bounds("labels-1-1").unwrap().size.width > px(100.));
    click("role-author", cx);
    finish(&view, &root, cx);
    click("role-reviewer", cx);
    finish(&view, &root, cx);
    click("sort-newest", cx);
    finish(&view, &root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert_eq!(state.config.mr_roles, [MrRole::Assignee, MrRole::Author]);
        assert_eq!(state.config.mr_sort, QueueSort::Newest);
        assert_eq!(
            state.queue.iter().map(|mr| mr.id.iid).collect::<Vec<_>>(),
            [2, 1]
        );
    });
    assert_eq!(service.preferences.lock().unwrap().len(), 3);
    assert_eq!(*executor.actions.lock().unwrap(), ["refresh", "refresh"]);
    assert_eq!(
        executor.refreshed_roles.lock().unwrap()[1],
        [MrRole::Assignee, MrRole::Author]
    );
    assert_eq!(service.tests.load(Ordering::SeqCst), 0);
    assert_eq!(service.saves.load(Ordering::SeqCst), 0);

    click("nav-settings", cx);
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).settings));
    assert!(cx.debug_bounds("gitlab-url").unwrap().size.height > px(0.));
    // GPUI 0.2.2 retains old debug bounds across frames; check that the old
    // filter location is no longer interactive after navigating to settings.
    click("role-reviewer", cx);
    draw(&root, cx);
    assert_eq!(service.preferences.lock().unwrap().len(), 3);
    click("nav-queue", cx);
    draw(&root, cx);
    click("role-assignee", cx);
    finish(&view, &root, cx);
    click("role-author", cx); // Last selected role cannot be removed.
    draw(&root, cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).config.mr_roles.clone()),
        [MrRole::Author]
    );
    assert_eq!(service.preferences.lock().unwrap().len(), 4);
}
#[gpui::test]
fn failed_queue_preference_save_retains_current_filters_and_rows(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![queue_fixture(1, "2026-11-05T12:00:00Z")],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(true)),
        executor.clone(),
        queue_config(),
    );
    let cx = &mut visual;
    click("sort-newest", cx);
    finish(&view, &root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert_eq!(state.config.mr_sort, QueueSort::Priority);
        assert_eq!(state.queue.len(), 1);
        assert!(state.status.contains("Cannot write queue preferences"));
    });
    assert!(executor.actions.lock().unwrap().is_empty());
}
#[gpui::test]
fn approval_requires_confirmation_and_sends_the_displayed_sha(cx: &mut TestAppContext) {
    let mr: MergeRequest = serde_json::from_value(serde_json::json!({
        "id":{"project":1,"iid":2}, "title":"Fix queue", "description":"",
        "project":"team/project", "author":"mara", "web_url":"https://example.invalid/mr/2",
        "sha":"a".repeat(40), "base_sha":"b".repeat(40), "created_at":"2026-11-06T12:00:00Z",
        "labels":[], "pipeline":null, "approvals":1, "changed_lines":3
    }))
    .unwrap();
    let executor = Arc::new(FakeExecutor {
        queue: vec![mr],
        ..Default::default()
    });
    let config = Config {
        gitlab_url: "https://gitlab.example.com".into(),
        // Keep the periodic refresh from racing with the clicks below.
        refresh_seconds: u64::MAX / 2,
        ..Default::default()
    };
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        config,
    );
    let cx = &mut visual;
    assert!(cx.debug_bounds("mr-actions").is_none());
    click("mr-1-2", cx);
    finish(&view, &root, cx);
    let queue = cx.debug_bounds("mr-queue").unwrap();
    let detail = cx.debug_bounds("mr-detail").unwrap();
    assert!(queue.bottom() <= detail.origin.y);
    let actions = cx.debug_bounds("mr-actions").unwrap();
    assert!(actions.bottom() <= detail.origin.y);
    assert!(actions.right() <= detail.right());
    click("approve", cx);
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).confirm));
    click("cancel-approval", cx);
    draw(&root, cx);
    assert!(!cx.update(|_, cx| view.read(cx).confirm));
    assert_eq!(*executor.actions.lock().unwrap(), ["select 2"]);

    click("approve", cx);
    draw(&root, cx);
    click("confirm-approval", cx);
    finish(&view, &root, cx);
    assert_eq!(
        *executor.actions.lock().unwrap(),
        [
            "select 2".to_string(),
            format!("approve {}", "a".repeat(40))
        ]
    );
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(!state.confirm);
        assert_eq!(state.queue[0].approvals, Some(2));
        assert_eq!(state.status, "Approved on GitLab.");
    });
    click("close-mr", cx);
    draw(&root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(state.selected_mr.is_none());
        assert!(state.detail.is_none());
    });
    assert!(cx.debug_bounds("mr-queue").unwrap().size.height > px(500.));
}
#[gpui::test]
fn failed_save_keeps_form_and_displays_keychain_error(cx: &mut TestAppContext) {
    let service = Arc::new(ControlledSettings::new(true));
    let (view, root, mut visual) = setup(cx, service.clone());
    let cx = &mut visual;
    fill(&view, &root, cx);
    click("save-settings", cx);
    service.gate.add_permits(1);
    finish(&view, &root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(state.feedback_error);
        assert!(state.feedback.contains("Keychain access denied"));
        assert_eq!(state.token.read(cx).value(), "synthetic-token");
        assert!(state.settings);
    });
}

#[gpui::test]
fn actions_and_error_remain_visible_with_logs_at_minimum_window_size(cx: &mut TestAppContext) {
    let service = Arc::new(ControlledSettings::new(true));
    let (view, root, mut visual) = setup(cx, service.clone());
    let cx = &mut visual;
    fill(&view, &root, cx);
    click("test-connection", cx);
    service.gate.add_permits(1);
    finish(&view, &root, cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    for _ in 0..4 {
        cx.draw(point(px(0.), px(0.)), size(px(1100.), px(720.)), |_, _| {
            root.clone()
        });
        cx.run_until_parked();
    }
    for selector in [
        "test-connection",
        "save-settings",
        "settings-feedback",
        "toggle-debug",
        "debug-panel",
    ] {
        let bounds = cx.debug_bounds(selector).expect(selector);
        assert!(
            bounds.origin.y >= px(0.) && bounds.bottom() <= px(720.),
            "Hidden feedback/control: {selector} {bounds:?}"
        );
        assert!(
            bounds.right() <= px(1100.),
            "Control overflows horizontally: {selector}"
        );
    }
}

#[gpui::test]
fn inbox_stacks_list_above_full_width_markdown_and_pins_actions_above_it(cx: &mut TestAppContext) {
    let service = Arc::new(ControlledSettings::new(false));
    let (view, root, mut visual) = setup(cx, service);
    let cx = &mut visual;
    let mr: mrde_core::models::MergeRequest = serde_json::from_value(serde_json::json!({
        "id":{"project":1,"iid":231}, "title":"Handle empty daily summaries", "description":"[Related issue](https://example.invalid/issues/42)\n\n## Summary\n\n**Fix** the `empty` result.\n\n- First item\n- Second item\n\n> Keep the behavior stable.\n\n```rust\nlet count = 0;\n```",
        "project":"orbit", "author":"mara", "web_url":"https://example.invalid/mr/231", "sha":"a".repeat(40), "base_sha":"b".repeat(40),
        "created_at":"2026-11-06T12:00:00Z", "labels":["P1"], "pipeline":"success", "approvals":1, "changed_lines":42
    })).unwrap();
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            state.queue = vec![mr.clone()];
            state.detail = Some(Box::new(mrde_core::models::MrDetail {
                mr,
                files: vec![
                    mrde_core::models::ChangedFile {
                        old_path: "src/lib.rs".into(),
                        new_path: "src/lib.rs".into(),
                        diff: "+a very large change\n".repeat(10_000),
                        too_large: false,
                        collapsed: false,
                    },
                    mrde_core::models::ChangedFile {
                        old_path: "README.md".into(),
                        new_path: "README.md".into(),
                        diff: String::new(),
                        too_large: false,
                        collapsed: false,
                    },
                ],
                issues: vec![],
                ssh_url: "git@example.invalid:orbit.git".into(),
            }));
            state.settings = false;
            cx.notify();
        })
    });
    cx.simulate_resize(size(px(1100.), px(720.)));
    for _ in 0..4 {
        cx.draw(point(px(0.), px(0.)), size(px(1100.), px(720.)), |_, _| {
            root.clone()
        });
        cx.run_until_parked();
    }
    let queue = cx.debug_bounds("mr-queue").unwrap();
    let detail = cx.debug_bounds("mr-detail").unwrap();
    let navigation = cx.debug_bounds("navigation").unwrap();
    let toolbar = cx.debug_bounds("queue-toolbar").unwrap();
    let actions = cx.debug_bounds("mr-actions").unwrap();
    assert!(queue.origin.x >= navigation.right());
    assert!(toolbar.bottom() <= queue.origin.y);
    assert!(queue.bottom() <= detail.origin.y);
    assert!(queue.size.height <= px(280.));
    assert!(actions.bottom() <= detail.origin.y);
    assert!(cx.debug_bounds("review-panel").is_none());
    assert!(cx.debug_bounds("disktree").is_none());
    assert!(cx.debug_bounds("changed-files-list").is_none());
    assert!(actions.right() <= detail.right());
    assert!(cx.debug_bounds("approve").unwrap().bottom() <= detail.origin.y);
    assert!(
        detail.size.width >= px(850.),
        "The reader must use the available inbox width"
    );
    assert!(
        cx.update(|_, cx| view.read(cx).job.is_none()),
        "Rendering should not trigger a network request"
    );
    let markdown = cx
        .debug_bounds("mr-markdown")
        .expect("MR description should use Markdown");
    assert!(
        markdown.size.height > px(100.),
        "Markdown blocks should have their own layout"
    );
    if markdown.origin.y + px(20.) >= detail.bottom() {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: detail.center(),
            delta: gpui::ScrollDelta::Pixels(point(
                px(0.),
                detail.origin.y - markdown.origin.y + px(20.),
            )),
            touch_phase: gpui::TouchPhase::Moved,
            ..Default::default()
        });
        cx.draw(point(px(0.), px(0.)), size(px(1100.), px(720.)), |_, _| {
            root.clone()
        });
    }
    let markdown = cx.debug_bounds("mr-markdown").unwrap();
    assert!(
        markdown.origin.y + px(20.) < detail.bottom(),
        "Markdown link must be in the reading viewport"
    );
    cx.simulate_click(
        gpui::point(markdown.origin.x + px(12.), markdown.origin.y + px(10.)),
        Modifiers::none(),
    );
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.invalid/issues/42"),
        "Markdown links should be interactive, not literal source text"
    );
    click("tab-files", cx);
    for _ in 0..4 {
        cx.draw(point(px(0.), px(0.)), size(px(1100.), px(720.)), |_, _| {
            root.clone()
        });
        cx.run_until_parked();
    }
    let files = cx
        .debug_bounds("changed-files-list")
        .expect("Changed files should have a compact list");
    assert!(
        files.size.height > px(30.) && files.size.height < px(120.),
        "A huge diff must not expand the filenames list"
    );
}

#[gpui::test]
fn pagination_renders_only_one_page_and_selects_the_correct_mr(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: (1..=1000)
            .map(|id| queue_fixture(id, "2026-11-05T12:00:00Z"))
            .collect(),
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        queue_config(),
    );
    let cx = &mut visual;
    assert!(cx.debug_bounds("mr-1-1").is_some());
    assert!(cx.debug_bounds("mr-1-25").is_some());
    assert!(
        cx.debug_bounds("mr-1-26").is_none(),
        "Off-page rows must not be rendered"
    );
    assert!(cx.debug_bounds("mr-1-1000").is_none());
    click("previous-page", cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).queue_page), 0);
    click("next-page", cx);
    draw(&root, cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).queue_page_range()), 25..50);
    click("mr-1-26", cx);
    finish(&view, &root, cx);
    assert_eq!(*executor.actions.lock().unwrap(), ["select 26"]);
    click("previous-page", cx);
    draw(&root, cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).queue_page), 0);
    click("next-page", cx);
    draw(&root, cx);
    click("sort-newest", cx);
    finish(&view, &root, cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).queue_page),
        0,
        "Sorting resets pagination"
    );
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            state.set_queue_page(39);
            let queue = state.queue[..27].to_vec();
            state
                .sender
                .send((state.generation, Ok(Outcome::Queue(queue))))
                .unwrap();
            state.poll(cx);
            assert_eq!(
                state.queue_page_range(),
                25..27,
                "Refresh clamps the last page"
            );
        })
    });
    draw(&root, cx);
    click("next-page", cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).queue_page), 1);
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            state
                .sender
                .send((state.generation, Ok(Outcome::Queue(vec![]))))
                .unwrap();
            state.poll(cx);
            assert_eq!(state.queue_page_range(), 0..0);
            assert_eq!(state.queue_page, 0);
        })
    });
}

#[gpui::test]
fn selecting_mr_loads_relative_markdown_image_through_the_image_client(cx: &mut TestAppContext) {
    let mut mr = queue_fixture(1, "2026-11-05T12:00:00Z");
    mr.web_url = "https://gitlab.example.com/team/app/-/merge_requests/1".into();
    mr.description =
        "![Sheet](/uploads/5f4dcc3b5aa765d61d8327deb882cf99/01-schedule-sheet.png)".into();
    let executor = Arc::new(FakeExecutor {
        queue: vec![mr],
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
    let expected = "https://gitlab.example.com/api/v4/projects/team%2Fapp/uploads/5f4dcc3b5aa765d61d8327deb882cf99/01-schedule-sheet.png";
    for _ in 0..200 {
        std::thread::sleep(Duration::from_millis(5));
        cx.run_until_parked();
        draw(&root, cx);
        if executor
            .actions
            .lock()
            .unwrap()
            .iter()
            .any(|a| a == &format!("image {expected}"))
        {
            break;
        }
    }
    assert!(
        executor
            .actions
            .lock()
            .unwrap()
            .iter()
            .any(|a| a == &format!("image {expected}")),
        "The Markdown image should request its project upload URL"
    );
    let resource = gpui::Resource::Uri(expected.to_string().into());
    let mut loaded = false;
    for _ in 0..200 {
        cx.run_until_parked();
        if let Some(result) =
            cx.update(|window, cx| window.get_asset::<gpui::ImgResourceLoader>(&resource, cx))
        {
            let image = result.expect("GPUI should decode the downloaded image");
            assert!(image.size(0).width.0 > 0);
            loaded = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(loaded, "Image decoding should finish");
    draw(&root, cx);
    let thumbnail = cx.debug_bounds("markdown-image").expect("rendered image");
    let approve = cx.debug_bounds("approve").unwrap();
    let scroll = cx.update(|_, cx| view.read(cx).detail_scroll.offset());
    click("markdown-image", cx);
    draw(&root, cx);
    let viewer = cx.debug_bounds("image-viewer").expect("modal image viewer");
    assert_eq!(viewer.size, size(px(1280.), px(900.)));
    let canvas = cx.debug_bounds("image-viewer-canvas").unwrap();
    assert!(canvas.size.width > thumbnail.size.width && canvas.size.height > thumbnail.size.height);
    assert!(cx.update(|_, cx| view.read(cx).image_viewer.read(cx).source().is_some()));
    assert!(
        cx.opened_url().is_none(),
        "Image clicks must stay in the application"
    );
    // An approval button behind the modal must not receive this click.
    cx.simulate_click(approve.center(), Modifiers::none());
    draw(&root, cx);
    assert!(!cx.update(|_, cx| view.read(cx).confirm));
    cx.simulate_keystrokes("tab shift-tab escape");
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).image_viewer.read(cx).source().is_none()));
    assert_eq!(
        cx.update(|_, cx| view.read(cx).detail_scroll.offset()),
        scroll
    );
    click("markdown-image", cx);
    draw(&root, cx);
    click("close-image", cx);
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).image_viewer.read(cx).source().is_none()));
    assert_eq!(
        executor
            .actions
            .lock()
            .unwrap()
            .iter()
            .filter(|action| action.starts_with("image "))
            .count(),
        1,
        "Enlarging an image should reuse its authenticated cached download"
    );

    assert!(
        cx.update(|_, cx| view
            .read(cx)
            .detail
            .as_ref()
            .unwrap()
            .mr
            .description
            .starts_with("![Sheet](/uploads/")),
        "Rendering must not rewrite the MR model or Claude context"
    );
}

fn selected_mr(description: &str) -> MergeRequest {
    serde_json::from_value(serde_json::json!({
        "id":{"project":1,"iid":2}, "title":"Edit an invoice schedule", "description":description,
        "project":"team/project", "author":"mara", "web_url":"https://example.invalid/mr/2",
        "sha":"a".repeat(40), "base_sha":"b".repeat(40), "created_at":"2026-11-06T12:00:00Z",
        "labels":[], "pipeline":null, "approvals":0, "changed_lines":53
    }))
    .unwrap()
}
fn quiet_config(spec_path: &str) -> Config {
    Config {
        gitlab_url: "https://gitlab.example.com".into(),
        // Keep the periodic refresh from racing with the clicks below.
        refresh_seconds: u64::MAX / 2,
        spec_path: spec_path.into(),
        ..Default::default()
    }
}
fn actions(executor: &FakeExecutor) -> Vec<String> {
    executor.actions.lock().unwrap().clone()
}

#[gpui::test]
fn selected_mr_actions_run_review_level_statistics_and_compliance(cx: &mut TestAppContext) {
    let specs = tempfile::tempdir().unwrap();
    let executor = Arc::new(FakeExecutor {
        queue: vec![selected_mr(
            "Intro\n\n```mermaid\nflowchart LR\n  A[Ticket] --> B[Review]\n```\n\nOutro",
        )],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        quiet_config(specs.path().to_str().unwrap()),
    );
    let cx = &mut visual;
    click("mr-1-2", cx);
    finish(&view, &root, cx);
    for selector in ["mr-actions", "mermaid-diagram"] {
        assert!(cx.debug_bounds(selector).is_some(), "missing {selector}");
    }
    assert!(
        cx.debug_bounds("disktree-map").is_none(),
        "no data before statistics"
    );

    click("tab-review", cx);
    draw(&root, cx);
    click("level-deep", cx);
    draw(&root, cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).level), ReviewLevel::Deep);
    click("run-review", cx);
    finish(&view, &root, cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).review_level),
        Some(ReviewLevel::Deep)
    );

    click_action("statistics", &root, cx);
    finish(&view, &root, cx);
    assert!(cx.debug_bounds("disktree-map").is_some());

    click_action("compliance-check", &root, cx);
    finish(&view, &root, cx);
    assert!(cx.debug_bounds("compliance-result").is_some());
    for selector in ["traceability", "requirement-0"] {
        assert!(cx.debug_bounds(selector).is_some(), "missing {selector}");
    }
    assert_eq!(
        actions(&executor),
        ["select 2", "review 2 Deep", "statistics", "compliance"]
    );
}

#[gpui::test]
fn analysis_runs_on_click_and_fills_structure_history_and_risks(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![selected_mr("Series")],
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
    for tab in ["tab-structure", "tab-history", "tab-risks"] {
        click(tab, cx);
        draw(&root, cx);
        assert!(
            cx.debug_bounds("analyze-prompt").is_some(),
            "{tab} explains how to analyze"
        );
    }
    assert_eq!(actions(&executor), ["select 2"], "nothing runs on its own");

    click("tab-structure", cx);
    draw(&root, cx);
    click_action("analyze-from-tab", &root, cx);
    finish(&view, &root, cx);
    for selector in ["structure-view", "symbols", "impact-0", "file-graph"] {
        assert!(cx.debug_bounds(selector).is_some(), "missing {selector}");
    }
    // GPUI 0.2.2 keeps debug bounds of elements no longer rendered, so the prompt's
    // disappearance is checked through the state that drives it.
    assert!(cx.update(|_, cx| view.read(cx).current_insights().is_some()));

    click("tab-history", cx);
    draw(&root, cx);
    for selector in [
        "history-view",
        "removed-0",
        "file-risks",
        "experts",
        "overlap-0",
    ] {
        assert!(cx.debug_bounds(selector).is_some(), "missing {selector}");
    }
    click("tab-risks", cx);
    draw(&root, cx);
    for selector in ["risks-view", "contracts-database", "contracts-http-api"] {
        assert!(cx.debug_bounds(selector).is_some(), "missing {selector}");
    }

    click("tab-statistics", cx);
    draw(&root, cx);
    click("disktree-risk", cx);
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).risk_colors));
    assert_eq!(actions(&executor), ["select 2", "analyze 2"]);
}

#[gpui::test]
fn analysis_of_another_commit_is_never_displayed(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![selected_mr("Series")],
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
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            let detail = state.detail.clone().unwrap();
            let mut stale = crate::demo::insights(&detail);
            stale.sha = "c".repeat(40);
            state.insights = Some((detail.mr.id.clone(), Box::new(stale)));
            state.tour = Some((detail.mr.id.clone(), "c".repeat(40), crate::demo::tour()));
            cx.notify();
        })
    });
    click("tab-structure", cx);
    draw(&root, cx);
    assert!(cx.debug_bounds("analyze-prompt").is_some());
    assert!(cx.debug_bounds("symbols").is_none());
    assert!(cx.debug_bounds("tour-step-0").is_none());
}

#[gpui::test]
fn tour_and_screenshots_run_only_on_click_and_only_when_configured(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![selected_mr("Series")],
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
    click("tab-structure", cx);
    draw(&root, cx);
    click("generate-tour", cx);
    finish(&view, &root, cx);
    for selector in ["tour-step-0", "tour-step-4", "tour-diagram"] {
        assert!(cx.debug_bounds(selector).is_some(), "missing {selector}");
    }

    click("tab-visual", cx);
    draw(&root, cx);
    click("capture-visual", cx);
    cx.run_until_parked();
    draw(&root, cx);
    assert_eq!(
        actions(&executor),
        ["select 2", "tour 2"],
        "no screenshot command configured"
    );
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            state.config.visual_command = "npm run screenshots".into();
            cx.notify();
        })
    });
    draw(&root, cx);
    click("capture-visual", cx);
    finish(&view, &root, cx);
    assert!(cx.debug_bounds("screen-0").is_some());
    assert_eq!(actions(&executor), ["select 2", "tour 2", "visual 2"]);
}

#[gpui::test]
fn compliance_check_requires_a_specification_folder(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![selected_mr("No spec yet")],
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
    click_action("compliance-check", &root, cx);
    finish(&view, &root, cx);
    assert_eq!(actions(&executor), ["select 2"]);
}

#[gpui::test]
fn worktrees_page_lists_worktrees_and_cleans_only_clean_ones(cx: &mut TestAppContext) {
    let entry = |iid, changes| mrde_git::WorktreeEntry {
        project: 1,
        iid,
        sha: "a".repeat(40),
        path: format!("/tmp/mrde/1-{iid}").into(),
        changes: Some(changes),
    };
    let executor = Arc::new(FakeExecutor {
        queue: vec![selected_mr("")],
        worktrees: vec![entry(2, 0), entry(3, 2)],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        quiet_config(""),
    );
    let cx = &mut visual;
    click("nav-worktrees", cx);
    finish(&view, &root, cx);
    assert!(cx.debug_bounds("worktree-0").is_some() && cx.debug_bounds("worktree-1").is_some());
    click("clean-worktree-1", cx); // has local work: disabled
    finish(&view, &root, cx);
    assert_eq!(actions(&executor), ["worktrees"]);
    click("clean-worktree-0", cx);
    finish(&view, &root, cx);
    assert_eq!(actions(&executor), ["worktrees", "remove 2"]);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert_eq!(state.worktrees.len(), 1);
        assert_eq!(state.status, "Worktree removed.");
    });
}

#[gpui::test]
fn version_is_shown_in_the_bottom_right_corner(cx: &mut TestAppContext) {
    let (_, _, mut visual) = setup(cx, Arc::new(ControlledSettings::new(false)));
    let version = visual.debug_bounds("app-version").expect("version label");
    assert!(
        version.right() <= px(1280.) && version.right() >= px(1200.),
        "{version:?}"
    );
    assert!(
        version.bottom() <= px(900.) && version.origin.y >= px(850.),
        "{version:?}"
    );
    assert_eq!(concat!("v", env!("CARGO_PKG_VERSION")), "v0.1.0");
}

#[gpui::test]
fn inbox_focus_tabs_and_adjacent_navigation_preserve_the_selected_mr(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: (1..=27)
            .map(|id| queue_fixture(id, "2026-11-05T12:00:00Z"))
            .collect(),
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        quiet_config(""),
    );
    let cx = &mut visual;
    click("mr-1-1", cx);
    finish(&view, &root, cx);
    let initial = cx.debug_bounds("mr-detail").unwrap();
    click("focus-reader", cx);
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).reader_focused));
    // GPUI 0.2 retains removed selectors in debug_bounds. Check the reader's
    // measured expansion and state rather than using stale selector absence.
    assert!(cx.debug_bounds("mr-detail").unwrap().size.height > initial.size.height + px(150.));
    click("tab-files", cx);
    draw(&root, cx);
    assert!(cx.debug_bounds("changed-files-list").is_some());
    assert!(cx.update(|_, cx| view.read(cx).detail_tab == DetailTab::Files));
    click("next-mr", cx);
    finish(&view, &root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert_eq!(state.detail.as_ref().unwrap().mr.id.iid, 2);
        assert!(state.reader_focused);
        assert!(state.detail_tab == DetailTab::Description);
    });
    assert!(cx.debug_bounds("mr-markdown").is_some());
    click("previous-mr", cx);
    finish(&view, &root, cx);
    assert_eq!(actions(&executor), ["select 1", "select 2", "select 1"]);
    click("focus-reader", cx);
    draw(&root, cx);
    assert!(cx.debug_bounds("mr-queue").is_some());
    // Adjacent navigation crosses the queue page boundary without losing selection.
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            state.open_mr(
                MrId {
                    project: 1,
                    iid: 25,
                },
                cx,
            )
        })
    });
    finish(&view, &root, cx);
    click("next-mr", cx);
    finish(&view, &root, cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).queue_page), 1);
    assert!(cx.debug_bounds("mr-1-26").is_some());
    click("approve", cx);
    draw(&root, cx);
    click("next-mr", cx);
    draw(&root, cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).detail.as_ref().unwrap().mr.id.iid),
        26
    );
    click("cancel-approval", cx);
    draw(&root, cx);
    click("close-mr", cx);
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).detail.is_none()));
    assert!(cx.debug_bounds("mr-queue").unwrap().size.height > px(500.));
}

#[gpui::test]
fn scoped_labels_have_adjoining_segments_in_queue_and_reader(cx: &mut TestAppContext) {
    let mut mr = queue_fixture(1, "2026-11-05T12:00:00Z");
    mr.labels = vec![Label {
        name: "Review::Dev".into(),
        color: Some("#428BCA".into()),
        text_color: Some("#FFFFFF".into()),
    }];
    let executor = Arc::new(FakeExecutor {
        queue: vec![mr],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor,
        quiet_config(""),
    );
    let cx = &mut visual;
    let check = |cx: &mut VisualTestContext| {
        let scope = cx.debug_bounds("label-scope-Review::Dev").unwrap();
        let value = cx.debug_bounds("label-value-Review::Dev").unwrap();
        assert!(scope.size.width > px(20.) && value.size.width > px(20.));
        assert_eq!(scope.origin.y, value.origin.y);
        assert_eq!(scope.right(), value.origin.x);
        assert_eq!(scope.size.height, value.size.height);
    };
    check(cx);
    click("mr-1-1", cx);
    finish(&view, &root, cx);
    click("focus-reader", cx);
    draw(&root, cx);
    check(cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).detail.as_ref().unwrap().mr.labels[0]
            .color
            .clone()),
        Some("#428BCA".into())
    );
}

#[gpui::test]
fn nested_reference_and_linked_images_open_the_viewer_without_opening_their_link(
    cx: &mut TestAppContext,
) {
    let upload = "/uploads/5f4dcc3b5aa765d61d8327deb882cf99/screenshot.png";
    for description in [
        format!("Before ![Inline]({upload}) after."),
        format!("> [![Linked]({upload})](https://example.invalid/external)"),
        format!("| Preview |\n| --- |\n| ![Referenced][shot] |\n\n[shot]: {upload}"),
    ] {
        let mut mr = queue_fixture(1, "2026-11-05T12:00:00Z");
        mr.web_url = "https://gitlab.example.com/team/app/-/merge_requests/1".into();
        mr.description = description;
        let executor = Arc::new(FakeExecutor {
            queue: vec![mr],
            ..Default::default()
        });
        let (view, root, mut visual) = setup_with(
            cx,
            Arc::new(ControlledSettings::new(false)),
            executor,
            quiet_config(""),
        );
        let visual = &mut visual;
        click("mr-1-1", visual);
        finish(&view, &root, visual);
        let uri = gpui::Resource::Uri("https://gitlab.example.com/api/v4/projects/team%2Fapp/uploads/5f4dcc3b5aa765d61d8327deb882cf99/screenshot.png".to_string().into());
        for _ in 0..200 {
            visual.run_until_parked();
            draw(&root, visual);
            if visual
                .update(|window, cx| window.get_asset::<gpui::ImgResourceLoader>(&uri, cx))
                .is_some()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        draw(&root, visual);
        click("markdown-image", visual);
        draw(&root, visual);
        assert!(visual.update(|_, cx| view.read(cx).image_viewer.read(cx).source().is_some()));
        assert!(visual.opened_url().is_none());
        visual.simulate_resize(size(px(1100.), px(720.)));
        visual.draw(point(px(0.), px(0.)), size(px(1100.), px(720.)), |_, _| {
            root.clone()
        });
        assert_eq!(
            visual.debug_bounds("image-viewer").unwrap().size,
            size(px(1100.), px(720.))
        );
        visual.simulate_keystrokes("escape");
        assert!(visual.update(|_, cx| view.read(cx).image_viewer.read(cx).source().is_none()));
    }
}

#[gpui::test]
fn image_viewer_download_failure_is_visible_and_can_be_closed(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        fail_images: true,
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor,
        quiet_config(""),
    );
    let cx = &mut visual;
    cx.update(|window, cx| {
        crate::image_viewer::open(
            "https://example.invalid/image.png".into(),
            "Unavailable image".into(),
            window,
            cx,
        )
    });
    for _ in 0..200 {
        cx.run_until_parked();
        draw(&root, cx);
        if cx.debug_bounds("image-viewer-error").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(cx.debug_bounds("image-viewer-error").is_some());
    click("close-image", cx);
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).image_viewer.read(cx).source().is_none()));
}

#[gpui::test]
fn detail_tabs_with_counters_fit_on_one_line_at_minimum_window_width(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![selected_mr("Series")],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor,
        quiet_config(""),
    );
    let cx = &mut visual;
    click("mr-1-2", cx);
    finish(&view, &root, cx);
    click("tab-review", cx);
    draw(&root, cx);
    click("run-review", cx);
    finish(&view, &root, cx);
    click("tab-structure", cx);
    draw(&root, cx);
    click_action("analyze-from-tab", &root, cx);
    finish(&view, &root, cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    draw(&root, cx);
    let tabs = cx.debug_bounds("detail-tabs").unwrap();
    let first = cx.debug_bounds("tab-description").unwrap();
    let last = cx.debug_bounds("tab-visual").unwrap();
    assert_eq!(first.origin.y, last.origin.y, "tabs wrapped: {tabs:?}");
    assert!(last.right() <= tabs.right());
}

#[gpui::test]
fn long_mr_titles_wrap_without_overlapping_the_metadata(cx: &mut TestAppContext) {
    let long_description = format!(
        "Ticket: web-app#2893\n\n## Scope\n\n{p}\n\n## What is delivered\n\n| Screen | Content |\n|---|---|\n{rows}\n## Where it lives\n\n```mermaid\nflowchart LR\n  A[fallbackDocumentTypes] --> B[documentTypes]\n  C[backend] --> D[Repository] --> E[UseCase] --> B\n```\n\n{p}\n",
        p = "The Documents tab lists the document types served by the backend, in the desktop order. ".repeat(6),
        rows = "| \"Add document\" type sheet | the list served by `GET /accounts/:id/document-types`, in server order, limited to the types the app can label |\n".repeat(5),
    );
    let mut mr = selected_mr(&long_description);
    mr.title = "web-app#2893 - Documents: the six document types match the desktop app and the real file name is shown".into();
    let executor = Arc::new(FakeExecutor {
        queue: vec![mr],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor,
        quiet_config(""),
    );
    let cx = &mut visual;
    cx.simulate_resize(size(px(1100.), px(900.)));
    draw(&root, cx);
    click("mr-1-2", cx);
    finish(&view, &root, cx);
    for (focused, width) in [
        (false, 1100.),
        (true, 1100.),
        (true, 1175.),
        (true, 1280.),
        (false, 1280.),
    ] {
        cx.simulate_resize(size(px(width), px(900.)));
        cx.update(|_, cx| {
            view.update(cx, |state, cx| {
                state.reader_focused = focused;
                cx.notify();
            })
        });
        draw(&root, cx);
        let title = cx.debug_bounds("mr-title").unwrap();
        let meta = cx.debug_bounds("mr-meta").unwrap();
        assert!(
            title.size.height >= px(60.),
            "the title wraps at {width}: {title:?}"
        );
        assert!(
            title.bottom() <= meta.origin.y,
            "title {title:?} overlaps {meta:?}"
        );
    }
}

fn finish_navigation(view: &Entity<Cockpit>, root: &Entity<Root>, cx: &mut VisualTestContext) {
    for _ in 0..200 {
        std::thread::sleep(Duration::from_millis(5));
        cx.run_until_parked();
        if cx.update(|_, cx| view.read(cx).navigation_job.is_none()) {
            draw(root, cx);
            return;
        }
    }
    panic!("Navigation did not complete independently of the operation");
}

#[gpui::test]
fn review_keeps_navigation_and_settings_editable_and_cannot_replace_another_mr(
    cx: &mut TestAppContext,
) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![
            queue_fixture(1, "2026-11-05T12:00:00Z"),
            queue_fixture(2, "2026-11-06T12:00:00Z"),
        ],
        operation_gate: Some(tokio::sync::Semaphore::new(0)),
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
    click("run-review", cx);
    draw(&root, cx);
    click("run-review", cx); // Must not start the same operation twice.
    click("nav-settings", cx);
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).settings));
    click("gitlab-token", cx);
    cx.simulate_input("synthetic-token");
    draw(&root, cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).token.read(cx).value()),
        "synthetic-token"
    );
    click("nav-worktrees", cx);
    finish_navigation(&view, &root, cx);
    assert!(cx.update(|_, cx| view.read(cx).worktrees_view));
    click("nav-queue", cx);
    draw(&root, cx);
    click("mr-1-2", cx);
    finish_navigation(&view, &root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert_eq!(state.detail.as_ref().unwrap().mr.id.iid, 2);
        assert!(state.job.is_some());
        assert!(state.review.is_none());
        assert!(state.status.contains("Running review"));
    });
    executor.operation_gate.as_ref().unwrap().add_permits(1);
    finish(&view, &root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert_eq!(state.detail.as_ref().unwrap().mr.id.iid, 2);
        assert!(state.review.is_none());
        assert!(state.detail_tab == DetailTab::Description);
        assert!(state.status.contains("another MR"));
    });
    assert_eq!(executor.operation_started.load(Ordering::SeqCst), 1);
}

#[gpui::test]
fn settings_operations_do_not_trap_navigation_or_return_to_settings_on_completion(
    cx: &mut TestAppContext,
) {
    for operation in [SettingsOperation::Test, SettingsOperation::Save] {
        let service = Arc::new(ControlledSettings::new(false));
        let executor = Arc::new(FakeExecutor {
            queue: vec![queue_fixture(1, "2026-11-05T12:00:00Z")],
            ..Default::default()
        });
        let (view, root, mut visual) = setup_with(cx, service.clone(), executor, queue_config());
        let cx = &mut visual;
        click("nav-settings", cx);
        draw(&root, cx);
        click(
            if operation == SettingsOperation::Test {
                "test-connection"
            } else {
                "save-settings"
            },
            cx,
        );
        draw(&root, cx);
        click("nav-queue", cx);
        draw(&root, cx);
        assert!(!cx.update(|_, cx| view.read(cx).settings));
        click("mr-1-1", cx);
        finish_navigation(&view, &root, cx);
        assert!(cx.update(|_, cx| view.read(cx).detail.is_some()));
        service.gate.add_permits(1);
        finish(&view, &root, cx);
        assert!(!cx.update(|_, cx| view.read(cx).settings));
        click("nav-settings", cx);
        draw(&root, cx);
        assert!(!cx.update(|_, cx| view.read(cx).feedback_error));
    }
}

#[gpui::test]
fn navigation_ignores_late_details_after_another_selection_and_close(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![
            queue_fixture(1, "2026-11-05T12:00:00Z"),
            queue_fixture(2, "2026-11-06T12:00:00Z"),
        ],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor,
        queue_config(),
    );
    let cx = &mut visual;
    click("mr-1-1", cx);
    finish(&view, &root, cx);
    let (old_generation, old_detail) = cx.update(|_, cx| {
        let state = view.read(cx);
        (state.navigation_generation, state.detail.clone().unwrap())
    });
    click("mr-1-2", cx);
    finish(&view, &root, cx);
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            state
                .navigation_sender
                .send((
                    old_generation,
                    Ok(Outcome::Detail(old_detail.clone(), None)),
                ))
                .unwrap();
            state.poll(cx);
            assert_eq!(state.detail.as_ref().unwrap().mr.id.iid, 2);
        })
    });
    let generation = cx.update(|_, cx| view.read(cx).navigation_generation);
    click("close-mr", cx);
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            state
                .navigation_sender
                .send((generation, Ok(Outcome::Detail(old_detail, None))))
                .unwrap();
            state.poll(cx);
            assert!(state.detail.is_none());
            assert!(state.selected_mr.is_none());
        })
    });
}

#[gpui::test]
fn startup_cache_can_wait_without_blocking_window_navigation(cx: &mut TestAppContext) {
    let (view, root, mut visual) = setup(cx, Arc::new(ControlledSettings::new(false)));
    let cx = &mut visual;
    let (release, gate) = std::sync::mpsc::channel();
    let ui_thread = std::thread::current().id();
    cx.update(|window, cx| {
        view.update(cx, |state, cx| {
            state.initialize(window, cx, move || {
                assert_ne!(std::thread::current().id(), ui_thread);
                gate.recv_timeout(Duration::from_secs(5))
                    .expect("test did not release the cache loader");
                (
                    queue_config(),
                    vec![queue_fixture(1, "2026-11-05T12:00:00Z")],
                    "Cache loaded".into(),
                    vec![queue_config()],
                )
            });
        })
    });
    draw(&root, cx);
    click("nav-settings", cx);
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).settings && view.read(cx).initializing));
    click("toggle-debug", cx);
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).logs_open));
    release.send(()).unwrap();
    for _ in 0..200 {
        std::thread::sleep(Duration::from_millis(5));
        cx.run_until_parked();
        if !cx.update(|_, cx| view.read(cx).initializing) {
            break;
        }
    }
    draw(&root, cx);
    cx.update(|_, cx| {
        let state = view.read(cx);
        assert!(!state.initializing);
        assert!(state.settings);
        assert_eq!(state.queue.len(), 1);
        assert_eq!(state.url.read(cx).value(), "https://gitlab.example.com");
    });
}

#[gpui::test]
fn large_file_list_only_builds_visible_rows_and_can_scroll_to_the_end(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![queue_fixture(1, "2026-11-05T12:00:00Z")],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor,
        queue_config(),
    );
    let cx = &mut visual;
    click("mr-1-1", cx);
    finish(&view, &root, cx);
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            state.detail.as_mut().unwrap().files = (0..3000)
                .map(|index| mrde_core::models::ChangedFile {
                    old_path: format!("src/file_{index}.rs"),
                    new_path: format!("src/file_{index}.rs"),
                    diff: "@@ -1 +1 @@\n-old\n+new\n".into(),
                    collapsed: false,
                    too_large: false,
                })
                .collect();
            state.presentation_revision += 1;
            state.reader_focused = true;
            state.show_detail_tab(DetailTab::Files);
            cx.notify();
        })
    });
    draw(&root, cx);
    assert!(cx.debug_bounds("changed-file-0").is_some());
    assert!(cx.debug_bounds("changed-file-2999").is_none());
    let list = cx.debug_bounds("changed-files-list").unwrap();
    let position = point(list.center().x, list.origin.y + px(30.));
    cx.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-200_000.))),
        touch_phase: gpui::TouchPhase::Moved,
        ..Default::default()
    });
    draw(&root, cx);
    assert!(
        cx.debug_bounds("changed-file-2999").is_some(),
        "The final file must stay accessible"
    );
}

fn pump_threads(view: &Entity<Cockpit>, root: &Entity<Root>, cx: &mut VisualTestContext) {
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(5));
        cx.update(|_, cx| {
            let threads = view.read(cx).threads.clone();
            threads.update(cx, |threads, cx| threads.poll(cx));
        });
        draw(root, cx);
    }
}
#[gpui::test]
fn resolver_keeps_navigation_available_and_stop_rejects_late_worker_events(
    cx: &mut TestAppContext,
) {
    use mrde_core::resolver::ResolverState;
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
    assert!(cx.update(|_, cx| view.read(cx).threads_view));
    assert_eq!(executor.resolver_runs.lock().unwrap().len(), 1);
    click("nav-settings", cx);
    draw(&root, cx);
    click("gitlab-token", cx);
    cx.simulate_input("synthetic-token");
    assert_eq!(
        cx.update(|_, cx| view.read(cx).token.read(cx).value().to_string()),
        "synthetic-token"
    );
    click("nav-threads", cx);
    draw(&root, cx);
    click("stop-resolver", cx);
    let mut late = executor.resolver_runs.lock().unwrap()[0].clone();
    late.state = ResolverState::Succeeded;
    late.sequence += 100;
    executor.resolver_updates.lock().unwrap()[0](mrde_resolver::Update::Thread(Box::new(late)));
    pump_threads(&view, &root, cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).threads.read(cx).records[0].state.clone()),
        ResolverState::Stopped
    );
    click("resume-resolver", cx);
    pump_threads(&view, &root, cx);
    assert_eq!(executor.resolver_runs.lock().unwrap().len(), 2);
    assert_eq!(executor.resolver_runs.lock().unwrap()[1].generation, 3);
}
#[gpui::test]
fn persisted_active_threads_require_manual_resume_after_relaunch(cx: &mut TestAppContext) {
    use crate::threads::Message;
    use mrde_core::resolver::ResolverState;
    let executor = Arc::new(FakeExecutor::default());
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        queue_config(),
    );
    let cx = &mut visual;
    let record = mrde_resolver::new_thread(
        queue_config().gitlab_url,
        MrId { project: 1, iid: 2 },
        "Interrupted repair".into(),
        "a".repeat(40),
        "{}".into(),
    );
    cx.update(|_, cx| {
        let threads = view.read(cx).threads.clone();
        threads.update(cx, |threads, cx| {
            threads
                .sender
                .send(Message::Loaded(1, Ok(vec![record])))
                .unwrap();
            threads.poll(cx);
        });
    });
    click("nav-threads", cx);
    draw(&root, cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).threads.read(cx).records[0].state.clone()),
        ResolverState::Interrupted
    );
    assert!(executor.resolver_runs.lock().unwrap().is_empty());
}

mod chrome;

mod ask;
mod projects;

mod scrolling;

mod risk;

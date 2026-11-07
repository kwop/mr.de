use crate::backend::{self, Action, Executor, Outcome};
use crate::{
    settings::{self, ConnectionSettings, Draft, SettingsService},
    theme::*,
};
use futures_util::FutureExt;
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Icon, IconName, Sizable,
    button::*,
    input::{Input, InputState},
};
use mrde_core::{
    config::{Config, MrRole, QueueSort},
    insights::{Insights, Tour, VisualReport},
    models::*,
};
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use inbox::DetailTab;
use labels::label_chip;

const QUEUE_PAGE_SIZE: usize = 25;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsOperation {
    Test,
    Save,
}

pub struct Cockpit {
    executor: Arc<dyn Executor>,
    image_viewer: Entity<crate::image_viewer::ImageViewer>,
    automatic: bool,
    refreshing: bool,
    settings_service: Option<Arc<dyn ConnectionSettings>>,
    settings_operation: Option<SettingsOperation>,
    feedback: String,
    feedback_error: bool,
    logs_open: bool,
    logs: std::collections::VecDeque<String>,
    progress_sender: mpsc::Sender<(u64, String)>,
    progress_receiver: mpsc::Receiver<(u64, String)>,
    runtime: Arc<tokio::runtime::Runtime>,
    config: Config,
    queue: Vec<MergeRequest>,
    queue_page: usize,
    queue_scroll: ScrollHandle,
    detail: Option<Box<MrDetail>>,
    review: Option<Review>,
    /// Level of the displayed review; `level` is the one chosen for the next run.
    review_level: Option<ReviewLevel>,
    level: ReviewLevel,
    stats: Option<(MrId, DiffStats)>,
    /// Render-time memo: the treemap is rebuilt only when its MR, commit, or source changes.
    disktree_cache: std::cell::RefCell<Option<disktree::Cached>>,
    compliance: Option<(MrId, String, Compliance)>,
    insights: Option<(MrId, Box<Insights>)>,
    tour: Option<(MrId, String, Tour)>,
    visual: Option<(MrId, String, VisualReport)>,
    /// Disktree colored by file risk instead of zone.
    risk_colors: bool,
    /// Real-application screenshot scenario; see `automation.rs`.
    automation: Option<automation::Automation>,
    worktrees_view: bool,
    worktrees: Vec<mrde_git::WorktreeEntry>,
    spec_path: Entity<InputState>,
    visual_command: Entity<InputState>,
    settings: bool,
    confirm: bool,
    status: String,
    url: Entity<InputState>,
    token: Entity<InputState>,
    cf_id: Entity<InputState>,
    cf_secret: Entity<InputState>,
    clear_cloudflare: bool,
    selected_mr: Option<MrId>,
    detail_scroll: ScrollHandle,
    detail_tab: DetailTab,
    reader_focused: bool,
    sender: mpsc::Sender<(u64, anyhow::Result<Outcome>)>,
    receiver: mpsc::Receiver<(u64, anyhow::Result<Outcome>)>,
    job: Option<tokio::task::JoinHandle<()>>,
    generation: u64,
    cancellable: bool,
    last_refresh: Instant,
    _poll: Task<()>,
}

impl Cockpit {
    pub fn new(
        runtime: Arc<tokio::runtime::Runtime>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if std::env::args().any(|arg| arg == "--ui-preview") {
            let config = Config::default();
            let mut cockpit = Self::with_state(
                runtime,
                Arc::new(backend::PreviewExecutor),
                Some(Arc::new(settings::PreviewSettings)),
                config,
                vec![],
                "UI preview — synthetic credentials; no network or Keychain access".into(),
                window,
                cx,
            );
            cockpit.apply_preview_scenario(cx);
            return cockpit;
        }
        let (config, status) = match backend::load_config() {
            Ok(config) => (config, "Ready".into()),
            Err(error) => (Config::default(), error.to_string()),
        };
        let queue = backend::cached_queue(&config).unwrap_or_default();
        let mut cockpit = Self::with_state(
            runtime,
            Arc::new(backend::SystemExecutor::default()),
            SettingsService::system()
                .ok()
                .map(|s| Arc::new(s) as Arc<dyn ConnectionSettings>),
            config,
            queue,
            status,
            window,
            cx,
        );
        cockpit.automation = automation::Automation::from_env();
        cockpit
    }
    #[allow(clippy::too_many_arguments)]
    fn with_state(
        runtime: Arc<tokio::runtime::Runtime>,
        executor: Arc<dyn Executor>,
        settings_service: Option<Arc<dyn ConnectionSettings>>,
        config: Config,
        queue: Vec<MergeRequest>,
        status: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let url =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://gitlab.example.com"));
        url.update(cx, |input, cx| {
            input.set_value(config.gitlab_url.clone(), window, cx)
        });
        let token = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("GitLab personal access token")
        });
        let cf_id = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("CF-Access-Client-Id (optional)")
        });
        let cf_secret = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("CF-Access-Client-Secret (optional)")
        });
        let spec_path =
            cx.new(|cx| InputState::new(window, cx).placeholder("~/code/my-project/specs"));
        spec_path.update(cx, |input, cx| {
            input.set_value(config.spec_path.clone(), window, cx)
        });
        let visual_command = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("pnpm --dir frontend exec playwright test visual")
        });
        visual_command.update(cx, |input, cx| {
            input.set_value(config.visual_command.clone(), window, cx)
        });
        cx.set_http_client(Arc::new(crate::images::ImageHttpClient::new(
            runtime.clone(),
            executor.clone(),
            config.clone(),
        )));
        let (sender, receiver) = mpsc::channel();
        let (progress_sender, progress_receiver) = mpsc::channel();
        let poll = cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(150)).await;
                if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            image_viewer: crate::image_viewer::install(window, cx),
            executor,
            automatic: false,
            refreshing: false,
            settings_service,
            settings_operation: None,
            feedback: String::new(),
            feedback_error: false,
            logs_open: false,
            logs: Default::default(),
            progress_sender,
            progress_receiver,
            runtime,
            settings: config.gitlab_url.is_empty(),
            selected_mr: None,
            detail_scroll: ScrollHandle::new(),
            detail_tab: DetailTab::Description,
            reader_focused: false,
            config,
            queue,
            queue_page: 0,
            queue_scroll: ScrollHandle::new(),
            detail: None,
            review: None,
            review_level: None,
            level: ReviewLevel::Standard,
            stats: None,
            disktree_cache: Default::default(),
            compliance: None,
            insights: None,
            tour: None,
            visual: None,
            risk_colors: false,
            automation: None,
            worktrees_view: false,
            worktrees: Vec::new(),
            spec_path,
            visual_command,
            confirm: false,
            status,
            url,
            token,
            cf_id,
            cf_secret,
            clear_cloudflare: false,
            sender,
            receiver,
            job: None,
            generation: 0,
            cancellable: false,
            last_refresh: Instant::now() - Duration::from_secs(86400),
            _poll: poll,
        }
    }
    /// Refresh is background work: only an explicit operation locks its controls.
    fn is_busy(&self) -> bool {
        self.job.is_some() && !self.refreshing
    }
    fn interrupt_refresh(&mut self) {
        if !self.refreshing {
            return;
        }
        if let Some(job) = self.job.take() {
            job.abort();
        }
        // A completed task may already have queued a response. Invalidate it too.
        self.generation += 1;
        self.refreshing = false;
        self.automatic = false;
        self.last_refresh = Instant::now();
        self.status = "Refresh cancelled to handle your action.".into();
        self.log("Refresh cancelled to handle your action");
    }
    fn start(&mut self, action: Action, message: &str, cx: &mut Context<Self>) {
        let refreshing = matches!(action, Action::Refresh);
        if self.is_busy() || (refreshing && self.job.is_some()) {
            return;
        }
        self.interrupt_refresh();
        self.refreshing = refreshing;
        self.status = message.into();
        self.log(message);
        self.automatic = false;
        self.cancellable = matches!(
            &action,
            Action::Review(..)
                | Action::Refresh
                | Action::Select(_)
                | Action::Statistics(_)
                | Action::Compliance(_)
                | Action::Worktrees
                | Action::Analyze(_)
                | Action::Tour(_)
                | Action::Visual(_)
        );
        self.generation += 1;
        let generation = self.generation;
        let sender = self.sender.clone();
        let config = self.config.clone();
        let executor = self.executor.clone();
        self.job = Some(self.runtime.spawn(async move {
            let result = std::panic::AssertUnwindSafe(executor.execute(config, action))
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!("Background operation stopped unexpectedly"))
                });
            let _ = sender.send((generation, result));
        }));
        cx.notify();
    }
    fn log(&mut self, message: &str) {
        if self.logs.len() >= 200 {
            self.logs.pop_front();
        }
        self.logs.push_back(format!(
            "{}  {}",
            chrono::Local::now().format("%H:%M:%S"),
            message
        ));
    }
    fn draft(&self, cx: &App) -> Draft {
        let mut config = self.config.clone();
        config.gitlab_url = self.url.read(cx).value().to_string();
        config.spec_path = self.spec_path.read(cx).value().trim().to_string();
        config.visual_command = self.visual_command.read(cx).value().trim().to_string();
        Draft {
            config,
            token: self.token.read(cx).value().to_string(),
            cf_id: self.cf_id.read(cx).value().to_string(),
            cf_secret: self.cf_secret.read(cx).value().to_string(),
            clear_cloudflare: self.clear_cloudflare,
        }
    }
    fn start_settings(&mut self, operation: SettingsOperation, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        let Some(service) = self.settings_service.clone() else {
            self.feedback = "Cannot locate the configuration directory.".into();
            self.feedback_error = true;
            cx.notify();
            return;
        };
        self.interrupt_refresh();
        let draft = self.draft(cx);
        let secrets = vec![
            draft.token.clone(),
            draft.cf_id.clone(),
            draft.cf_secret.clone(),
        ];
        self.settings_operation = Some(operation);
        self.feedback_error = false;
        self.feedback = if operation == SettingsOperation::Test {
            "Testing connection…"
        } else {
            "Saving settings…"
        }
        .into();
        self.status = self.feedback.clone();
        self.log(&self.feedback.clone());
        self.generation += 1;
        self.cancellable = false;
        let generation = self.generation;
        let sender = self.sender.clone();
        let progress_sender = self.progress_sender.clone();
        let progress = Arc::new(move |message: String| {
            let _ = progress_sender.send((generation, message));
        });
        self.job = Some(self.runtime.spawn(async move {
            let task = async {
                match operation {
                    SettingsOperation::Test => service
                        .test(draft, progress)
                        .await
                        .map(Outcome::ConnectionTested),
                    SettingsOperation::Save => service
                        .save(draft, progress)
                        .await
                        .map(Outcome::SettingsSaved),
                }
            };
            let result = std::panic::AssertUnwindSafe(task)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Settings operation stopped unexpectedly")))
                .map_err(|error| {
                    anyhow::anyhow!(settings::redact(&format!("{error:#}"), &secrets))
                });
            let _ = sender.send((generation, result));
        }));
        cx.notify();
    }
    fn poll(&mut self, cx: &mut Context<Self>) {
        while let Ok((generation, message)) = self.progress_receiver.try_recv() {
            if generation == self.generation {
                self.log(&message);
                self.status = message;
                cx.notify();
            }
        }
        while let Ok((generation, result)) = self.receiver.try_recv() {
            if generation != self.generation {
                continue;
            }
            self.job = None;
            self.refreshing = false;
            self.last_refresh = Instant::now();
            let settings_operation = self.settings_operation.take();
            let automatic = std::mem::take(&mut self.automatic);
            let succeeded = result.is_ok();
            match result {
                Err(error) => {
                    self.status = format!("{error:#}");
                    if settings_operation.is_some() {
                        self.feedback = self.status.clone();
                        self.feedback_error = true;
                    }
                    self.log(&format!("ERROR  {}", self.status));
                    // Periodic failures stay in the status line instead of reopening the panel.
                    if !automatic {
                        self.logs_open = true;
                    }
                }
                Ok(Outcome::ConnectionTested(user)) => {
                    self.feedback =
                        format!("Connected as {user}. Test passed — settings have not been saved.");
                    self.feedback_error = false;
                    self.status = "Connection test passed".into();
                    self.log("OK  Connection test passed; no settings written");
                }
                Ok(Outcome::SettingsSaved(config)) => {
                    self.executor.reset();
                    cx.set_http_client(Arc::new(crate::images::ImageHttpClient::new(
                        self.runtime.clone(),
                        self.executor.clone(),
                        config.clone(),
                    )));
                    self.config = config;
                    self.clear_cloudflare = false;
                    self.settings = true;
                    self.queue.clear();
                    self.set_queue_page(0);
                    self.selected_mr = None;
                    self.detail = None;
                    self.review = None;
                    self.status = "Settings saved".into();
                    self.feedback = "Saved. Your credentials are in macOS Keychain and your configuration is ready.".into();
                    self.feedback_error = false;
                    self.log("OK  Settings saved");
                    self.last_refresh = Instant::now() - Duration::from_secs(86400);
                }
                Ok(Outcome::QueuePreferencesSaved(config)) => {
                    let roles_changed = config.mr_roles != self.config.mr_roles;
                    self.config = config;
                    self.set_queue_page(0);
                    mrde_core::scoring::sort_queue(
                        &mut self.queue,
                        self.config.mr_sort,
                        chrono::Utc::now(),
                    );
                    self.status = "Queue preferences saved".into();
                    if roles_changed {
                        self.queue.clear();
                        self.set_queue_page(0);
                        self.detail = None;
                        self.selected_mr = None;
                        self.review = None;
                        self.confirm = false;
                        self.refresh(false, cx);
                    }
                }
                Ok(Outcome::Queue(queue)) => {
                    if let Some(detail) = &self.detail
                        && !queue
                            .iter()
                            .any(|mr| mr.id == detail.mr.id && mr.sha == detail.mr.sha)
                    {
                        self.detail = None;
                        self.selected_mr = None;
                        self.review = None;
                        self.confirm = false;
                    }
                    self.status = format!(
                        "{} open merge requests · updated {}",
                        queue.len(),
                        chrono::Local::now().format("%H:%M:%S")
                    );
                    self.queue = queue;
                    self.set_queue_page(self.queue_page);
                }
                Ok(Outcome::DetailWith(detail, review, restored)) => {
                    let (id, sha) = (detail.mr.id.clone(), detail.mr.sha.clone());
                    self.insights = restored
                        .insights
                        .filter(|i| i.sha == sha)
                        .map(|i| (id.clone(), Box::new(i)));
                    if let Some((_, insights)) = &self.insights
                        && let Some(stats) = insights.stats.clone()
                    {
                        self.stats = Some((id.clone(), stats));
                    }
                    self.tour = restored.tour.map(|t| (id.clone(), sha.clone(), t));
                    self.visual = restored.visual.map(|v| (id.clone(), sha.clone(), v));
                    self.apply_detail(detail, review);
                }
                Ok(Outcome::Insights(id, insights)) => {
                    self.status = format!(
                        "Analysis ready: {} symbols, {} contracts, {} patterns.",
                        insights.symbols.len(),
                        insights.contracts.len(),
                        insights.patterns.len()
                    );
                    for warning in &insights.warnings {
                        self.log(&format!("WARN  {warning}"));
                    }
                    if let Some(stats) = insights.stats.clone() {
                        self.stats = Some((id.clone(), stats));
                    }
                    self.insights = Some((id, insights));
                    if !matches!(
                        self.detail_tab,
                        DetailTab::History | DetailTab::Risks | DetailTab::Statistics
                    ) {
                        self.show_detail_tab(DetailTab::Structure);
                    }
                }
                Ok(Outcome::Tour(id, sha, tour)) => {
                    self.status = format!("Guided tour ready: {} steps.", tour.steps.len());
                    self.tour = Some((id, sha, tour));
                    self.show_detail_tab(DetailTab::Structure);
                }
                Ok(Outcome::Visual(id, sha, report)) => {
                    self.status =
                        format!("Screenshots compared: {} screens.", report.screens.len());
                    self.visual = Some((id, sha, report));
                    self.show_detail_tab(DetailTab::Visual);
                }
                Ok(Outcome::Detail(detail, review)) => {
                    self.apply_detail(detail, review);
                }
                Ok(Outcome::Review(level, review)) => {
                    self.show_detail_tab(DetailTab::Review);
                    self.review = Some(review);
                    self.review_level = Some(level);
                    self.status = format!("{} review ready.", level.label());
                }
                Ok(Outcome::Statistics(id, stats)) => {
                    self.status = format!("Statistics ready: {} files.", stats.files.len());
                    self.stats = Some((id, stats));
                    self.show_detail_tab(DetailTab::Statistics);
                }
                Ok(Outcome::Compliance(id, sha, result)) => {
                    self.status = "Compliance check ready.".into();
                    self.compliance = Some((id, sha, result));
                    self.show_detail_tab(DetailTab::Checks);
                }
                Ok(Outcome::Worktrees(worktrees, message)) => {
                    self.status =
                        message.unwrap_or_else(|| format!("{} worktrees.", worktrees.len()));
                    self.worktrees = worktrees;
                }
                Ok(Outcome::Approved(id, count)) => {
                    if let Some(detail) = self.detail.as_mut().filter(|d| d.mr.id == id) {
                        detail.mr.approvals = count;
                    }
                    if let Some(mr) = self.queue.iter_mut().find(|mr| mr.id == id) {
                        mr.approvals = count;
                    }
                    self.confirm = false;
                    self.status = "Approved on GitLab.".into();
                }
                Ok(Outcome::Done(message)) => {
                    self.status = message;
                    self.confirm = false;
                }
            }
            if succeeded && settings_operation.is_none() {
                self.log(&format!("OK  {}", self.status));
            }
            cx.notify();
        }
        if self.job.is_none()
            && !self.settings
            && !self.confirm
            && !self.config.gitlab_url.is_empty()
            && self.last_refresh.elapsed()
                >= Duration::from_secs(self.config.refresh_seconds.max(30))
        {
            self.refresh(true, cx);
        }
        self.drive_automation(cx);
    }
    fn apply_detail(&mut self, detail: Box<MrDetail>, review: Option<(ReviewLevel, Review)>) {
        self.selected_mr = Some(detail.mr.id.clone());
        self.detail_tab = DetailTab::Description;
        self.detail_scroll.set_offset(point(px(0.), px(0.)));
        // Results only stay visible for the exact commit they describe.
        let (id, sha) = (detail.mr.id.clone(), detail.mr.sha.clone());
        if self
            .stats
            .as_ref()
            .is_some_and(|(i, s)| *i != id || s.sha != sha)
        {
            self.stats = None;
        }
        if self
            .compliance
            .as_ref()
            .is_some_and(|(i, s, _)| *i != id || *s != sha)
        {
            self.compliance = None;
        }
        if self
            .insights
            .as_ref()
            .is_some_and(|(i, insights)| *i != id || insights.sha != sha)
        {
            self.insights = None;
        }
        if self
            .tour
            .as_ref()
            .is_some_and(|(i, s, _)| *i != id || *s != sha)
        {
            self.tour = None;
        }
        if self
            .visual
            .as_ref()
            .is_some_and(|(i, s, _)| *i != id || *s != sha)
        {
            self.visual = None;
        }
        self.detail = Some(detail);
        self.review_level = review.as_ref().map(|(level, _)| *level);
        self.review = review.map(|(_, review)| review);
        self.confirm = false;
        self.status = "MR loaded. Pre-review runs only on click.".into();
    }
    fn refresh(&mut self, automatic: bool, cx: &mut Context<Self>) {
        if self.job.is_some() || self.confirm {
            return;
        }
        self.last_refresh = Instant::now();
        let generation = self.generation;
        self.start(Action::Refresh, "Refreshing GitLab merge requests…", cx);
        if self.generation != generation {
            self.automatic = automatic;
        }
    }
    fn change_queue_preferences(
        &mut self,
        roles: Vec<MrRole>,
        sort: QueueSort,
        cx: &mut Context<Self>,
    ) {
        if self.is_busy() || self.confirm || roles.is_empty() {
            return;
        }
        let Some(service) = self.settings_service.clone() else {
            self.status = "Configuration storage is unavailable.".into();
            cx.notify();
            return;
        };
        self.interrupt_refresh();
        self.status = "Saving queue preferences…".into();
        self.log("Saving queue preferences");
        self.automatic = false;
        self.cancellable = false;
        self.generation += 1;
        let generation = self.generation;
        let sender = self.sender.clone();
        self.job = Some(self.runtime.spawn(async move {
            let result = std::panic::AssertUnwindSafe(service.save_queue_preferences(roles, sort))
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Queue preferences could not be saved")))
                .map(Outcome::QueuePreferencesSaved);
            let _ = sender.send((generation, result));
        }));
        cx.notify();
    }
    fn queue_page_count(&self) -> usize {
        self.queue.len().div_ceil(QUEUE_PAGE_SIZE).max(1)
    }
    fn set_queue_page(&mut self, page: usize) {
        self.queue_page = page.min(self.queue_page_count() - 1);
        self.queue_scroll.set_offset(point(px(0.), px(0.)));
    }
    fn queue_page_range(&self) -> std::ops::Range<usize> {
        let start = self.queue_page.min(self.queue_page_count() - 1) * QUEUE_PAGE_SIZE;
        start..(start + QUEUE_PAGE_SIZE).min(self.queue.len())
    }
    fn queue_pagination(&self, cx: &mut Context<Self>) -> AnyElement {
        let range = self.queue_page_range();
        let disabled = self.is_busy() || self.confirm;
        div()
            .debug_selector(|| "queue-pagination".into())
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_size(px(12.)).text_color(rgb(MUTED)).child(
                if self.queue.is_empty() {
                    "0 results".into()
                } else {
                    format!("{}–{} of {}", range.start + 1, range.end, self.queue.len())
                },
            ))
            .child(
                div().debug_selector(|| "previous-page".into()).child(
                    Button::new("previous-page")
                        .small()
                        .ghost()
                        .label("Previous")
                        .disabled(disabled || self.queue_page == 0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_queue_page(this.queue_page.saturating_sub(1));
                            cx.notify();
                        })),
                ),
            )
            .child(
                div().debug_selector(|| "next-page".into()).child(
                    Button::new("next-page")
                        .small()
                        .ghost()
                        .label("Next")
                        .disabled(disabled || self.queue_page + 1 >= self.queue_page_count())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_queue_page(this.queue_page + 1);
                            cx.notify();
                        })),
                ),
            )
            .into_any_element()
    }
    fn queue_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.is_busy() || self.confirm || self.config.gitlab_url.is_empty();
        let mut roles = div().flex().items_center().gap_1().child(
            div()
                .text_size(px(12.))
                .text_color(rgb(MUTED))
                .mr_1()
                .child("Show"),
        );
        for (index, role) in MrRole::ALL.into_iter().enumerate() {
            let checked = self.config.mr_roles.contains(&role);
            roles = roles.child(
                div()
                    .debug_selector(move || format!("role-{}", role.label().to_lowercase()))
                    .child(
                        Button::new(("queue-role", index))
                            .small()
                            .ghost()
                            .label(role.label())
                            .when(checked, |b| b.bg(rgb(SELECTED)).text_color(rgb(HEADING)))
                            .disabled(disabled || (checked && self.config.mr_roles.len() == 1))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let roles = MrRole::ALL
                                    .into_iter()
                                    .filter(|r| {
                                        if *r == role {
                                            !checked
                                        } else {
                                            this.config.mr_roles.contains(r)
                                        }
                                    })
                                    .collect();
                                this.change_queue_preferences(roles, this.config.mr_sort, cx);
                            })),
                    ),
            );
        }
        let mut sorts = div().flex().items_center().gap_1().child(
            div()
                .text_size(px(12.))
                .text_color(rgb(MUTED))
                .mr_1()
                .child("Sort"),
        );
        for (index, sort) in QueueSort::ALL.into_iter().enumerate() {
            let label = match sort {
                QueueSort::Priority => "Priority",
                QueueSort::Newest => "Newest",
                QueueSort::Oldest => "Oldest",
                QueueSort::Smallest => "Smallest diff",
            };
            sorts = sorts.child(
                div()
                    .debug_selector(move || format!("sort-{sort:?}").to_lowercase())
                    .child(
                        Button::new(("queue-sort", index))
                            .small()
                            .ghost()
                            .label(label)
                            .when(self.config.mr_sort == sort, |b| {
                                b.bg(rgb(SELECTED)).text_color(rgb(HEADING))
                            })
                            .disabled(disabled || self.config.mr_sort == sort)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.change_queue_preferences(
                                    this.config.mr_roles.clone(),
                                    sort,
                                    cx,
                                );
                            })),
                    ),
            );
        }
        div()
            .debug_selector(|| "queue-toolbar".into())
            .bg(rgb(TOOLBAR))
            .flex_shrink_0()
            .px_5()
            .py_2()
            .flex()
            .flex_col()
            .gap_2()
            .border_b_1()
            .border_color(rgb(PANEL_LINE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_color(rgb(HEADING))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Merge requests"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(MUTED))
                                    .child(format!("{} open", self.queue.len())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(self.queue_pagination(cx)),
                    )
                    .child(
                        div().debug_selector(|| "refresh-queue".into()).child(
                            Button::new("refresh")
                                .small()
                                .ghost()
                                .icon(IconName::Redo)
                                .label("Refresh")
                                .loading(self.refreshing)
                                .disabled(disabled || self.refreshing)
                                .on_click(cx.listener(|this, _, _, cx| this.refresh(false, cx))),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(roles)
                    .child(sorts),
            )
            .into_any_element()
    }
    fn settings_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.is_busy();
        let row = |label: &'static str, hint: &'static str, control: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap_8()
                .py_5()
                .child(
                    // GPUI measures text min-content as one unwrapped line; without
                    // min_w_0 a long hint pushes its control out of the column.
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(label)
                        .child(div().text_sm().text_color(rgb(MUTED)).child(hint)),
                )
                .child(div().w(px(340.)).flex_shrink_0().child(control))
        };
        let field = |label: &'static str,
                     hint: &'static str,
                     input: &Entity<InputState>,
                     selector: &'static str| {
            row(
                label,
                hint,
                div()
                    .debug_selector(move || selector.into())
                    .child(
                        Input::new(input)
                            .disabled(busy)
                            .bg(rgb(FIELD))
                            .text_size(px(UI_SIZE))
                            .rounded(px(7.))
                            .h(px(34.)),
                    )
                    .into_any_element(),
            )
        };
        let content = div().w_full().max_w(px(920.)).px(px(56.)).py(px(42.)).flex().flex_col()
            .child(div().text_size(px(20.)).text_color(rgb(HEADING)).font_weight(FontWeight::MEDIUM).mb_2().child("GitLab"))
            .child(div().text_sm().text_color(rgb(MUTED)).mb_8().child("Connect your review workspace."))
            .child(field("Instance URL", "Your self-hosted GitLab instance.", &self.url, "gitlab-url"))
            .child(field("Personal access token", "API access for merge requests and approvals.", &self.token, "gitlab-token"))
            .child(div().text_sm().text_color(rgb(MUTED)).pb_6().child("Leave a secret blank to keep its saved value. Entered values remain visible as masked text."))
            .child(div().h(px(1.)).bg(rgb(LINE)).my_3())
            .child(div().text_size(px(15.)).text_color(rgb(HEADING)).font_weight(FontWeight::MEDIUM).mt_6().mb_2().child("Compliance"))
            .child(field("Specification folder", "Folder searched by the compliance check, for example my-project/specs.", &self.spec_path, "spec-path"))
            .child(field("Screenshot command", "Optional. Runs in the base and MR worktrees and writes PNG files to $MRDE_SCREENSHOTS_DIR for the visual comparison.", &self.visual_command, "visual-command"))
            .child(div().h(px(1.)).bg(rgb(LINE)).my_3())
            .child(div().text_size(px(15.)).text_color(rgb(HEADING)).font_weight(FontWeight::MEDIUM).mt_6().mb_2().child("Cloudflare Access"))
            .child(div().text_sm().text_color(rgb(MUTED)).mb_2().child("Optional service token for instances behind Zero Trust."))
            .child(field("Client ID", "CF-Access-Client-Id", &self.cf_id, "cf-id"))
            .child(field("Client secret", "CF-Access-Client-Secret", &self.cf_secret, "cf-secret"))
            .child(div().flex().justify_end().pb_5().child(Button::new("clear-cf").ghost().small()
                .label(if self.clear_cloudflare { "Undo removal of saved Cloudflare credentials" } else { "Remove saved Cloudflare credentials" })
                .disabled(busy).on_click(cx.listener(|this, _, _, cx| { this.clear_cloudflare = !this.clear_cloudflare; cx.notify(); }))))
            ;
        let mut controls = div()
            .flex_shrink_0()
            .border_t_1()
            .border_color(rgb(LINE))
            .px(px(56.))
            .py_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div().debug_selector(|| "test-connection".into()).child(
                            Button::new("test-connection")
                                .label("Test connection")
                                .loading(self.settings_operation == Some(SettingsOperation::Test))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_settings(SettingsOperation::Test, cx)
                                })),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "save-settings".into()).child(
                            Button::new("save-settings")
                                .primary()
                                .label("Save")
                                .loading(self.settings_operation == Some(SettingsOperation::Save))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_settings(SettingsOperation::Save, cx)
                                })),
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child("Testing does not save your settings."),
                    ),
            );
        if !self.feedback.is_empty() {
            controls = controls.child(
                div()
                    .debug_selector(|| "settings-feedback".into())
                    .mt_4()
                    .p_3()
                    .rounded(px(8.))
                    .bg(rgb(if self.feedback_error {
                        ERROR_BG
                    } else {
                        SUCCESS_BG
                    }))
                    .text_sm()
                    .text_color(rgb(if self.feedback_error { ERROR } else { SUCCESS }))
                    .child(self.feedback.clone()),
            );
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(content),
            )
            .child(controls)
            .into_any_element()
    }
    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .debug_selector(|| "navigation".into())
            .text_size(px(13.))
            .w(px(SIDEBAR_WIDTH))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(rgb(PANEL_LINE))
            .bg(rgb(SIDEBAR))
            .px_3()
            .py_5()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .px_3()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .mb_2()
                    .child("Workspace"),
            )
            .child(
                div()
                    .id("nav-queue")
                    .debug_selector(|| "nav-queue".into())
                    .h_9()
                    .px_3()
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .bg(rgb(if !self.settings && !self.worktrees_view {
                        SELECTED
                    } else {
                        SIDEBAR
                    }))
                    .cursor_pointer()
                    .child(Icon::new(IconName::Inbox).size_4())
                    .child("Merge requests")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.is_busy() {
                            this.settings = false;
                            this.worktrees_view = false;
                            cx.notify();
                        }
                    })),
            )
            .child(
                div()
                    .id("nav-worktrees")
                    .debug_selector(|| "nav-worktrees".into())
                    .h_9()
                    .px_3()
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .bg(rgb(if self.worktrees_view && !self.settings {
                        SELECTED
                    } else {
                        SIDEBAR
                    }))
                    .cursor_pointer()
                    .child(Icon::new(IconName::Folder).size_4())
                    .child("Worktrees")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.is_busy() && !this.confirm {
                            this.settings = false;
                            this.worktrees_view = true;
                            this.start(Action::Worktrees, "Listing worktrees…", cx);
                        }
                    })),
            )
            .child(
                div()
                    .px_3()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .mt_8()
                    .mb_2()
                    .child("Settings"),
            )
            .child(
                div()
                    .id("nav-settings")
                    .debug_selector(|| "nav-settings".into())
                    .h_9()
                    .px_3()
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .bg(rgb(if self.settings { SELECTED } else { SIDEBAR }))
                    .cursor_pointer()
                    .child(Icon::new(IconName::Settings).size_4())
                    .child("GitLab")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.is_busy() {
                            this.settings = true;
                            this.worktrees_view = false;
                            cx.notify();
                        }
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .px_3()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Local workspace"),
            )
            .child(div().px_3().text_sm().child("macOS · SSH"))
            .into_any_element()
    }
    fn debug_panel(&self) -> AnyElement {
        div()
            .id("debug-log")
            .debug_selector(|| "debug-panel".into())
            .h(px(168.))
            .flex_shrink_0()
            .bg(rgb(PAGE))
            .border_t_1()
            .border_color(rgb(LINE))
            .overflow_y_scroll()
            .px_5()
            .py_3()
            .font_family(CODE_FONT)
            .text_size(px(11.))
            .text_color(rgb(MUTED))
            .children(
                self.logs
                    .iter()
                    .map(|line| div().py_1().child(line.clone())),
            )
            .into_any_element()
    }
    fn queue_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div()
            .id("queue")
            .debug_selector(|| "mr-queue".into())
            .track_scroll(&self.queue_scroll)
            .bg(rgb(QUEUE))
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if self.queue.is_empty() {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(rgb(MUTED))
                    .child(
                        Icon::new(IconName::Inbox)
                            .size(px(24.))
                            .text_color(rgb(SUBTLE)),
                    )
                    .child(if self.job.is_some() {
                        "Loading merge requests…"
                    } else if self.config.gitlab_url.is_empty() {
                        "Connect GitLab in Settings to load your merge requests."
                    } else {
                        "No open merge requests match your roles."
                    }),
            );
        }
        for mr in &self.queue[self.queue_page_range()] {
            let id = mr.id.clone();
            let selected = self.selected_mr.as_ref() == Some(&id)
                || self.detail.as_ref().is_some_and(|d| d.mr.id == id);
            // One line per MR: priority first, then as many labels as fit in a row.
            let mut labels = div()
                .flex()
                .min_w_0()
                .overflow_hidden()
                .items_center()
                .gap_1()
                .debug_selector({
                    let id = id.clone();
                    move || format!("labels-{}-{}", id.project, id.iid)
                })
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(SUBTLE))
                        .mr_1()
                        .child(format!("!{}", id.iid)),
                );
            const QUEUE_LABELS: usize = 5;
            let ordered = labels::display_order(mr.all_labels());
            for label in ordered.iter().take(QUEUE_LABELS) {
                labels = labels.child(label_chip(label));
            }
            if ordered.len() > QUEUE_LABELS {
                labels = labels.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(rgb(SUBTLE))
                        .child(format!("+{}", ordered.len() - QUEUE_LABELS)),
                );
            }
            let age = (chrono::Utc::now() - mr.created_at).num_hours().max(0);
            let age = if age < 1 {
                "<1h ago".into()
            } else if age < 24 {
                format!("{age}h ago")
            } else {
                format!("{}d ago", age / 24)
            };
            list =
                list.child(
                    div()
                        .id(SharedString::from(format!("mr-{}-{}", id.project, id.iid)))
                        .debug_selector({
                            let id = id.clone();
                            move || format!("mr-{}-{}", id.project, id.iid)
                        })
                        .flex_shrink_0()
                        .min_h(px(56.))
                        .relative()
                        .pl(px(14.))
                        .pr_5()
                        .py_2()
                        .border_b_1()
                        .border_color(rgb(LINE))
                        .bg(rgb(if selected { SELECTED } else { QUEUE }))
                        .hover(move |style| {
                            style.bg(rgb(if selected { SELECTED_HOVER } else { HOVER }))
                        })
                        .cursor_pointer()
                        .flex()
                        .items_start()
                        .gap_4()
                        .child(div().absolute().left_0().top_0().bottom_0().w(px(3.)).bg(
                            if selected {
                                rgb(INFO).into()
                            } else {
                                transparent_black()
                            },
                        ))
                        .child(
                            div()
                                .w(px(140.))
                                .flex_shrink_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(rgb(INK))
                                        .one_line()
                                        .child(crate::ellipsis::ellipsis(
                                            mr.project
                                                .rsplit('/')
                                                .next()
                                                .unwrap_or(&mr.project)
                                                .to_string(),
                                        )),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(MUTED))
                                        .one_line()
                                        .child(crate::ellipsis::ellipsis(mr.author.clone())),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .line_height(px(20.))
                                        .text_color(rgb(HEADING))
                                        .one_line()
                                        .child(crate::ellipsis::ellipsis(mr.title.clone())),
                                )
                                .child(labels),
                        )
                        .child(
                            div()
                                .w(px(188.))
                                .flex_shrink_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_2()
                                        .text_size(px(12.))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .text_color(rgb(MUTED))
                                                .child(pipeline_dot(mr.pipeline.as_deref()))
                                                .child(
                                                    mr.pipeline
                                                        .clone()
                                                        .unwrap_or_else(|| "No pipeline".into()),
                                                ),
                                        )
                                        .child(div().text_color(rgb(MUTED)).child(age)),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(SUBTLE))
                                        .child(inbox::mr_metrics(mr.changed_lines, mr.approvals)),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_mr(id.clone(), cx);
                        })),
                );
        }
        list.into_any_element()
    }
    fn description_view(
        &self,
        detail: &MrDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_4()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .debug_selector(|| "mr-markdown".into())
                    .child(crate::markdown::view(
                        SharedString::from(format!(
                            "mr-{}-{}-description",
                            detail.mr.id.project, detail.mr.id.iid
                        )),
                        &detail.mr.description,
                        Some((&self.config.gitlab_url, &detail.mr.web_url)),
                        window,
                        cx,
                    )),
            );
        if detail.issues.is_empty() {
            panel = panel.child("No related GitLab issues found.");
        }
        for (index, issue) in detail.issues.iter().enumerate() {
            let url = issue.web_url.clone();
            panel = panel
                .child(
                    Button::new(("issue", index))
                        .label(issue.title.clone())
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
                .child(crate::markdown::view(
                    SharedString::from(format!("issue-{}", issue.web_url)),
                    &issue.description,
                    Some((&self.config.gitlab_url, &issue.web_url)),
                    window,
                    cx,
                ));
        }
        panel.into_any_element()
    }
    fn files_view(&self, detail: &MrDetail) -> AnyElement {
        let stats = self.file_stats(detail);
        let widest = stats.iter().map(|f| f.changed()).max().unwrap_or(1).max(1);
        let files = div()
            .debug_selector(|| "changed-files-list".into())
            .w_full()
            .flex()
            .flex_col()
            .children(detail.files.iter().enumerate().map(|(index, file)| {
                let stat = stats.iter().find(|s| s.path == file.new_path);
                let (directory, name) = match file.new_path.rsplit_once('/') {
                    Some((directory, name)) => (format!("{directory}/"), name.to_string()),
                    None => (String::new(), file.new_path.clone()),
                };
                let renamed = file.old_path != file.new_path;
                let bar = stat.map(|s| {
                    let width = 64. * s.changed() as f32 / widest as f32;
                    let added = if s.changed() == 0 {
                        0.
                    } else {
                        width * s.added as f32 / s.changed() as f32
                    };
                    div()
                        .w(px(64.))
                        .h(px(6.))
                        .flex_shrink_0()
                        .rounded(px(3.))
                        .bg(rgb(FIELD))
                        .flex()
                        .child(div().w(px(added)).h_full().bg(rgb(SUCCESS)))
                        .child(div().w(px(width - added)).h_full().bg(rgb(ERROR)))
                });
                div()
                    .debug_selector(move || format!("changed-file-{index}"))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        Icon::new(IconName::File)
                            .size(px(14.))
                            .flex_shrink_0()
                            .text_color(rgb(SUBTLE)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .one_line()
                            .font_family(CODE_FONT)
                            .text_size(px(CODE_SIZE))
                            .child(
                                crate::ellipsis::ellipsis(format!("{directory}{name}")).colors([
                                    (0..directory.len(), rgb(SUBTLE).into()),
                                    (
                                        directory.len()..directory.len() + name.len(),
                                        rgb(INK).into(),
                                    ),
                                ]),
                            ),
                    )
                    .when(renamed, |row| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(rgb(WARNING))
                                .child("renamed"),
                        )
                    })
                    .when(file.too_large || file.collapsed, |row| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(rgb(WARNING))
                                .child("truncated by GitLab"),
                        )
                    })
                    .when_some(stat, |row, stat| {
                        row.child(
                            div()
                                .w(px(96.))
                                .flex_shrink_0()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .font_family(CODE_FONT)
                                .text_size(px(11.))
                                .child(
                                    div()
                                        .text_color(rgb(SUCCESS))
                                        .child(format!("+{}", stat.added)),
                                )
                                .child(
                                    div()
                                        .text_color(rgb(ERROR))
                                        .child(format!("−{}", stat.removed)),
                                ),
                        )
                    })
                    .children(bar)
            }));
        files.into_any_element()
    }
    fn review_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.detail.is_none() {
            return div().into_any_element();
        }
        let mut panel = div()
            .debug_selector(|| "review-panel".into())
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.review_controls(cx));
        panel = panel.child(
            div()
                .border_t_1()
                .border_color(rgb(LINE))
                .pt_4()
                .flex()
                .items_center()
                .gap_2()
                .text_color(rgb(HEADING))
                .child(
                    Icon::new(IconName::Bot)
                        .size(px(15.))
                        .text_color(rgb(CLAUDE)),
                )
                .child(match self.review_level {
                    Some(level) => format!("Pre-review · {}", level.label()),
                    None => "Pre-review".into(),
                }),
        );
        if let Some(review) = &self.review {
            let document = self
                .detail
                .as_ref()
                .map(|d| format!("{}-{}-{}", d.mr.id.project, d.mr.id.iid, d.mr.sha))
                .unwrap_or_else(|| "none".into());
            panel = panel
                .child(crate::markdown::view(
                    SharedString::from(format!("review-{document}-summary")),
                    &review.summary,
                    self.detail
                        .as_ref()
                        .map(|d| (self.config.gitlab_url.as_str(), d.mr.web_url.as_str())),
                    window,
                    cx,
                ))
                .child(section_label("Intention"))
                .child(crate::markdown::view(
                    SharedString::from(format!("review-{document}-intention")),
                    &review.intention,
                    self.detail
                        .as_ref()
                        .map(|d| (self.config.gitlab_url.as_str(), d.mr.web_url.as_str())),
                    window,
                    cx,
                ))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child(format!("Confidence  {:.0}%", review.confidence * 100.)),
                )
                .child(section_label("Risks"));
            if review.risks.is_empty() {
                panel = panel.child(
                    div()
                        .flex()
                        .gap_2()
                        .items_center()
                        .text_color(rgb(MUTED))
                        .child(
                            Icon::new(IconName::Check)
                                .size(px(14.))
                                .text_color(rgb(SUCCESS)),
                        )
                        .child("No risks reported"),
                );
            }
            for (index, risk) in review.risks.iter().enumerate() {
                let (severity, color) = match risk.severity {
                    Severity::Critical => ("Critical", ERROR),
                    Severity::High => ("High", CLAUDE),
                    Severity::Medium => ("Medium", WARNING),
                    Severity::Low => ("Low", INFO),
                };
                let mut row = div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .w(px(14.))
                            .h(px(20.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(status_dot(color, false)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(color))
                                    .child(severity),
                            )
                            .child(crate::markdown::view(
                                SharedString::from(format!("review-{document}-risk-{index}")),
                                &risk.description,
                                self.detail.as_ref().map(|d| {
                                    (self.config.gitlab_url.as_str(), d.mr.web_url.as_str())
                                }),
                                window,
                                cx,
                            )),
                    );
                if let Some(file) = &risk.file {
                    row = div().flex().flex_col().gap_1().child(row).child(
                        div()
                            .pl(px(26.))
                            .font_family(CODE_FONT)
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(file.clone()),
                    );
                }
                panel = panel.child(row);
            }
            panel = panel.child(section_label("Read first"));
            for file in &review.priority_files {
                panel = panel.child(
                    div()
                        .flex()
                        .items_start()
                        .gap_3()
                        .child(
                            Icon::new(IconName::File)
                                .size(px(14.))
                                .text_color(rgb(SUBTLE)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_family(CODE_FONT)
                                .text_size(px(CODE_SIZE))
                                .child(file.clone()),
                        ),
                );
            }
        } else {
            panel = panel.child(
                div()
                    .text_color(rgb(MUTED))
                    .line_height(px(21.))
                    .child("Choose a review level, then click Run review to start Claude."),
            );
        }
        panel.into_any_element()
    }
}
/// Shared inset surface: visible boundary, with content clipped to its rounded corners.
fn panel_surface(background: u32) -> Div {
    div()
        .bg(rgb(background))
        .border_1()
        .border_color(rgb(PANEL_LINE))
        .rounded(px(10.))
        .overflow_hidden()
}

fn status_dot(color: u32, hollow: bool) -> Div {
    div()
        .size(px(6.))
        .flex_shrink_0()
        .rounded_full()
        .border_1()
        .border_color(rgb(color))
        .bg(if hollow {
            transparent_black()
        } else {
            rgb(color).into()
        })
}
fn pipeline_dot(status: Option<&str>) -> Div {
    let (color, hollow) = match status {
        Some("success") => (SUCCESS, false),
        Some("running" | "pending" | "created" | "waiting_for_resource") => (WARNING, true),
        Some("failed") => (ERROR, false),
        Some("manual" | "scheduled") => (INFO, true),
        _ => (SUBTLE, true),
    };
    status_dot(color, hollow)
}
fn section_label(text: &'static str) -> Div {
    div()
        .mt_4()
        .text_size(px(13.))
        .text_color(rgb(MUTED))
        .child(text)
}

impl Drop for Cockpit {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.abort();
        }
    }
}
impl Render for Cockpit {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .px_3()
            .pt_2()
            .flex()
            .flex_col();
        if self.settings {
            main = main.child(
                panel_surface(READING)
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.settings_view(cx)),
            );
        } else if self.worktrees_view {
            main = main.child(
                panel_surface(READING)
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.worktrees_view(cx)),
            );
        } else {
            let reading_open = self.selected_mr.is_some() || self.detail.is_some();
            let queue = panel_surface(QUEUE)
                .debug_selector(|| "queue-panel".into())
                .min_h_0()
                .flex()
                .flex_col()
                .when(reading_open, |d| {
                    d.h(px(260.)).max_h(relative(0.40)).flex_shrink_0()
                })
                .when(!reading_open, |d| d.flex_1())
                .child(self.queue_toolbar(cx))
                .child(div().flex_1().min_h_0().child(self.queue_view(cx)));
            let mut inbox = div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_2()
                .when(!reading_open || !self.reader_focused, |d| d.child(queue));
            if reading_open {
                let reading = panel_surface(READING)
                    .debug_selector(|| "reading-pane".into())
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(self.reader_toolbar(cx))
                    .when(self.confirm, |d| d.child(self.approval_confirmation(cx)))
                    .child(self.detail_view(window, cx));
                inbox = inbox.child(reading);
            }
            main = main.child(inbox);
        }
        if self.logs_open {
            main = main.child(self.debug_panel());
        }
        let mut footer = div()
            .h(px(38.))
            .flex_shrink_0()
            .bg(rgb(PAGE))
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(self.status.clone()),
            )
            .child(
                div().debug_selector(|| "toggle-debug".into()).child(
                    Button::new("toggle-debug")
                        .ghost()
                        .small()
                        .label(if self.logs_open {
                            "Hide debug logs"
                        } else {
                            "Debug logs"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.logs_open = !this.logs_open;
                            cx.notify();
                        })),
                ),
            );
        if self.job.is_some() && self.cancellable {
            footer = footer.child(
                Button::new("cancel-operation")
                    .small()
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(job) = this.job.take() {
                            job.abort();
                        }
                        this.generation += 1;
                        this.refreshing = false;
                        this.automatic = false;
                        this.last_refresh = Instant::now();
                        this.status = "Operation cancelled.".into();
                        cx.notify();
                    })),
            );
        }
        footer = footer.child(
            div()
                .debug_selector(|| "app-version".into())
                .flex_shrink_0()
                .text_xs()
                .text_color(rgb(SUBTLE))
                .child(concat!("v", env!("CARGO_PKG_VERSION"))),
        );
        main = main.child(footer);
        let titlebar = div()
            .h(px(42.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .window_control_area(WindowControlArea::Drag)
            .child(
                div()
                    .w(px(SIDEBAR_WIDTH))
                    .h_full()
                    .flex_shrink_0()
                    .bg(rgb(SIDEBAR))
                    .border_r_1()
                    .border_color(rgb(PANEL_LINE))
                    .pl(px(98.))
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(rgb(SUBTLE))
                    .child("mr.de"),
            )
            .child(
                div()
                    .px_5()
                    .text_size(px(12.))
                    .text_color(rgb(SUBTLE))
                    .child(if self.settings {
                        "GitLab settings".to_string()
                    } else {
                        self.detail
                            .as_ref()
                            .map(|d| d.mr.project.clone())
                            .unwrap_or_else(|| "Review workspace".into())
                    }),
            );
        div()
            .relative()
            .size_full()
            .bg(rgb(PAGE))
            .text_color(rgb(INK))
            .text_size(px(UI_SIZE))
            .font_family(UI_FONT)
            .line_height(px(20.))
            .flex()
            .flex_col()
            .child(titlebar)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar(cx))
                    .child(main),
            )
            .child(self.image_viewer.clone())
    }
}

mod actions;
mod automation;
mod disktree;
mod history;
mod inbox;
mod labels;
mod preview;
mod risks;
mod structure;
mod visual;
mod widgets;
mod worktrees;

#[cfg(test)]
mod tests;

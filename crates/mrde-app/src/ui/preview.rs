//! `--ui-preview` scenarios: put the interface in a given state with demo data, for visual
//! checks and screenshots. `MRDE_SCENARIO` examples: `queue`, `detail:files`,
//! `detail:review`, `focus:statistics`, `confirm`, `worktrees`, `settings`, `logs`, `open`.
use super::{Cockpit, context::ContextLayout, inbox::DetailTab};
use crate::demo;
use gpui::{Context, Window};
use mrde_core::{config::Config, models::ReviewLevel};
use std::time::Instant;

pub(super) fn tab(key: &str) -> DetailTab {
    match key {
        "changes" => DetailTab::Changes,
        "files" => DetailTab::Files,
        "review" => DetailTab::Review,
        "statistics" => DetailTab::Statistics,
        "checks" => DetailTab::Checks,
        "structure" => DetailTab::Structure,
        "history" => DetailTab::History,
        "risks" => DetailTab::Risks,
        "visual" => DetailTab::Visual,
        _ => DetailTab::Description,
    }
}

impl Cockpit {
    pub(super) fn apply_preview_scenario(&mut self, cx: &mut Context<Self>) {
        let scenario = std::env::var("MRDE_SCENARIO").unwrap_or_default();
        self.apply_preview_scenario_named(&scenario);
        self.threads
            .update(cx, |threads, _| threads.use_preview_terminal());
        // `ask`: the questions thread opened by "Ask Claude", with the local preview shell.
        if scenario == "ask" {
            self.apply_preview_scenario_named("detail:description");
            if let Some(detail) = self.detail.clone() {
                let directory = std::env::temp_dir();
                let ready = crate::backend::AskReady::new(
                    &detail,
                    directory.clone(),
                    directory.clone(),
                    &directory.join("context.md"),
                );
                self.threads
                    .update(cx, |threads, cx| threads.ask(ready, cx));
                self.threads_view = true;
                self.status = "UI preview — demo data, no network · scenario: ask".into();
            }
        }
        if scenario.starts_with("threads") {
            self.threads_view = true;
            self.settings = false;
            self.threads.update(cx, |threads, cx| {
                threads.preview(scenario == "threads:terminal", cx);
                if scenario == "threads:closed" {
                    threads.preview_closed(cx);
                }
            });
            if scenario == "threads:terminal"
                && std::env::var_os("MRDE_TERMINAL_NAVIGATE").is_some()
            {
                cx.spawn(async move |this, cx| {
                    gpui::Timer::after(std::time::Duration::from_millis(2200)).await;
                    let _ = this.update(cx, |this, cx| {
                        this.threads_view = false;
                        this.settings = true;
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }
    pub(super) fn apply_preview_scenario_named(&mut self, scenario: &str) {
        let scenario = scenario.to_string();
        self.config.gitlab_url = demo::INSTANCE.into();
        self.config.name = "Platform".into();
        self.projects = vec![
            self.config.clone(),
            Config {
                id: "community".into(),
                name: "Community".into(),
                provider: mrde_core::config::Provider::Github,
                gitlab_url: "https://github.com".into(),
                ..Default::default()
            },
        ];
        self.projects_open = scenario == "projects";
        self.config.spec_path = "~/code/platform/specs".into();
        self.config.visual_command = "pnpm --dir web exec playwright test visual".into();
        self.settings = false;
        self.queue = demo::queue();
        self.last_refresh = Instant::now();
        let (name, argument) = scenario.split_once(':').unwrap_or((&scenario, ""));
        let iid = std::env::var("MRDE_SCENARIO_MR")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(231);
        let open = |this: &mut Self| {
            let Some(detail) = this
                .queue
                .iter()
                .find(|mr| mr.id.iid == iid)
                .and_then(|mr| demo::detail(&mr.id))
            else {
                return;
            };
            this.selected_mr = Some(detail.mr.id.clone());
            this.stats = Some((detail.mr.id.clone(), demo::stats(&detail)));
            this.compliance = Some((
                detail.mr.id.clone(),
                detail.mr.sha.clone(),
                demo::compliance(),
            ));
            this.review = Some(demo::review());
            // `MRDE_SCENARIO_FRESH=1` (or `detail:fresh`) shows tabs before any analysis.
            let fresh = argument == "fresh" || std::env::var("MRDE_SCENARIO_FRESH").is_ok();
            if !fresh {
                let id = detail.mr.id.clone();
                let report = demo::risk(&detail);
                // The queue's miniature radar shows the same cached profile.
                if let Some(mr) = this.queue.iter_mut().find(|mr| mr.id == id) {
                    mr.risk = Some(mrde_core::risk::RiskSummary::from(&report));
                }
                this.risk = Some((id.clone(), Box::new(report)));
                this.risk_baseline = Some((id.clone(), demo::baseline()));
                this.insights = Some((id.clone(), Box::new(demo::insights(&detail))));
                this.tour = Some((id.clone(), detail.mr.sha.clone(), demo::tour()));
                if let Ok(report) = demo::visual() {
                    this.visual = Some((id, detail.mr.sha.clone(), report));
                }
            }
            this.risk_colors = std::env::var("MRDE_SCENARIO_RISK").is_ok();
            this.review_level = Some(ReviewLevel::Deep);
            if let Ok(axis) = std::env::var("MRDE_SCENARIO_AXIS") {
                this.risk_axis = mrde_core::risk::Axis::ALL
                    .into_iter()
                    .find(|a| a.id() == axis);
            }
            if std::env::var("MRDE_SCENARIO_FILE").as_deref() == Ok("first")
                && let Some(file) = demo::stats(&detail).files.first()
            {
                this.risk_selected_file = Some(file.path.clone());
                this.risk_file = Some((
                    detail.mr.id.clone(),
                    detail.mr.sha.clone(),
                    demo::file_context(&file.path),
                ));
            }
            this.detail = Some(Box::new(detail));
            this.detail_tab = tab(argument);
        };
        match name {
            "detail" => open(self),
            "focus" => {
                open(self);
                self.reader_focused = true;
            }
            "confirm" => {
                open(self);
                self.confirm = true;
            }
            "worktrees" => {
                self.worktrees_view = true;
                self.worktrees = demo::worktrees();
            }
            "settings" => {
                self.settings = true;
                if argument == "github" {
                    self.config = self.projects[1].clone();
                    self.queue.clear();
                }
            }
            "logs" => {
                open(self);
                self.logs_open = true;
                for line in [
                    "Loading MR details…",
                    "OK  MR loaded. Pre-review runs only on click.",
                ] {
                    self.log(line);
                }
            }
            _ => {}
        }
        if let Some(offset) = std::env::var("MRDE_SCENARIO_SCROLL")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
        {
            self.detail_scroll
                .set_offset(gpui::point(gpui::px(0.), gpui::px(-offset)));
        }
        self.context_pinned = match std::env::var("MRDE_SCENARIO_CONTEXT").as_deref() {
            Ok("docked") => Some(ContextLayout::Docked),
            Ok("overlay") => Some(ContextLayout::Overlay),
            Ok("rail") => Some(ContextLayout::Rail),
            Ok("closed") => Some(ContextLayout::Hidden),
            _ => None,
        };
        if let Ok(label) = std::env::var("MRDE_SCENARIO_LABEL") {
            self.filter.labels = vec![label];
        }
        self.status = format!("UI preview — demo data, no network · scenario: {scenario}");
    }
    /// Inputs need the window: `MRDE_SCENARIO_SEARCH=<text>` fills the search, and the
    /// `open` scenario shows the "Open MR…" dialog with a reference.
    pub(super) fn apply_preview_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.project_inputs(&self.config.clone(), window, cx);
        // `settings:github`: the settings of the GitHub project.
        if std::env::var("MRDE_SCENARIO").as_deref() == Ok("settings:github")
            && let Some(mut project) = self
                .projects
                .iter()
                .find(|p| p.provider == mrde_core::config::Provider::Github)
                .cloned()
        {
            project.group = "acme/platform".into();
            self.project_inputs(&project, window, cx);
        }
        // Explicit preview-only fixture: permits offline visual checks with locally measured data.
        if let Some(file) = std::env::var_os("MRDE_SCENARIO_RISK_FILE") {
            let read = cx.background_executor().spawn(async move {
                serde_json::from_str::<mrde_core::risk::RiskReport>(
                    &std::fs::read_to_string(file).map_err(anyhow::Error::from)?,
                )
                .map_err(anyhow::Error::from)
            });
            cx.spawn(async move |this, cx| {
                if let Ok(report) = read.await {
                    let _ = this.update(cx, |this, cx| {
                        if let Some(detail) = &mut this.detail {
                            detail.mr.sha = report.profile.head_sha.clone();
                            detail.mr.base_sha = report.profile.base_sha.clone();
                            detail.mr.title = "Local risk report".into();
                            detail.mr.labels.clear();
                            detail.mr.ticket_labels.clear();
                            let id = detail.mr.id.clone();
                            this.risk_baseline = None;
                            this.apply_risk(id, Box::new(report));
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
        }

        if let Ok(query) = std::env::var("MRDE_SCENARIO_SEARCH") {
            self.search
                .update(cx, |input, cx| input.set_value(query.clone(), window, cx));
            self.filter.query = query;
        }
        if let Ok(scenario) = std::env::var("MRDE_SCENARIO")
            && scenario.starts_with("image")
        {
            crate::image_viewer::open(
                format!("{}/preview/portrait.svg", demo::INSTANCE).into(),
                "Portrait image".into(),
                window,
                cx,
            );
            let zoom = match scenario.as_str() {
                "image:max" => 8.,
                "image:zoom" => 4.,
                _ => 1.,
            };
            // Zoom only after decoding, as a real user does, so captures keep the focal point.
            // Position the preview pointer over the image to inspect the magnifier.
            let handle = window.window_handle();
            let viewer = self.image_viewer.clone();
            cx.spawn(async move |_, cx| {
                gpui::Timer::after(std::time::Duration::from_millis(1000)).await;
                let _ = handle.update(cx, |_, window, cx| {
                    let size = window.viewport_size();
                    viewer.update(cx, |viewer, cx| {
                        viewer.set_zoom(zoom, window, cx);
                        viewer.pointer_move(
                            &gpui::MouseMoveEvent {
                                position: gpui::point(size.width / 2., size.height / 2.),
                                pressed_button: None,
                                modifiers: gpui::Modifiers::none(),
                            },
                            window,
                            cx,
                        )
                    });
                });
            })
            .detach();
        }
        if std::env::var("MRDE_SCENARIO").as_deref() == Ok("open") {
            self.show_open_dialog(window, cx);
            self.open_input.update(cx, |input, cx| {
                input.set_value(
                    "https://gitlab.example.com/acme/platform/-/merge_requests/238",
                    window,
                    cx,
                )
            });
        }
    }
}

//! Project navigation sits above provider-specific views.
use super::*;
use mrde_core::config::Provider;

impl Cockpit {
    pub(super) fn project_switcher(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.initializing || self.settings_operation.is_some();
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_1()
            .mb_4()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .px_2()
                    .child(crate::i18n::t("PROJECT")),
            )
            .child(
                div().debug_selector(|| "project-switcher".into()).child(
                    Button::new("project-switcher")
                        .ghost()
                        .w_full()
                        .label(self.config.name.clone())
                        .icon(IconName::ChevronDown)
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.projects_open = !this.projects_open;
                            cx.notify();
                        })),
                ),
            );
        if self.projects_open {
            for (index, project) in self.projects.iter().enumerate() {
                let id = project.id.clone();
                let selector = format!("project-{id}");
                panel = panel.child(
                    div().debug_selector(move || selector.clone()).child(
                        Button::new(("project", index))
                            .ghost()
                            .w_full()
                            .label(format!("{} · {}", project.name, project.provider.label()))
                            .when(project.id == self.config.id, |b| b.bg(rgb(SELECTED)))
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.switch_project(id.clone(), window, cx);
                            })),
                    ),
                );
            }
            panel = panel.child(
                div().debug_selector(|| "new-project".into()).child(
                    Button::new("new-project")
                        .ghost()
                        .w_full()
                        .icon(IconName::Plus)
                        .label(crate::i18n::t("New project"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let config = Config {
                                id: mrde_resolver::Uuid::new_v4().to_string(),
                                name: String::new(),
                                ..Default::default()
                            };
                            this.project_inputs(&config, window, cx);
                            this.editing_project = Some(config);
                            this.settings = true;
                            this.worktrees_view = false;
                            this.threads_view = false;
                            this.projects_open = false;
                            this.feedback.clear();
                            cx.notify();
                        })),
                ),
            );
        }
        panel.into_any_element()
    }
    pub(super) fn project_inputs(
        &mut self,
        config: &Config,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, value) in [
            (&self.url, config.gitlab_url.clone()),
            (&self.project_name, config.name.clone()),
            (&self.project_group, config.group.clone()),
            (&self.project_refresh, config.refresh_seconds.to_string()),
            (&self.project_claude, config.claude_path.clone()),
            (&self.spec_path, config.spec_path.clone()),
            (&self.visual_command, config.visual_command.clone()),
            (&self.token, String::new()),
            (&self.cf_id, String::new()),
            (&self.cf_secret, String::new()),
        ] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.language_draft = self.config.languages;
        self.draft_provider = config.provider;
        self.clear_cloudflare = false;
    }
    fn switch_project(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.initializing || self.settings_operation.is_some() {
            return;
        }
        let Some(service) = self.settings_service.clone() else {
            return;
        };
        self.projects_open = false;
        self.initializing = true;
        self.interrupt_refresh();
        self.cancel_navigation();
        // In-flight reviews/approvals finish against their captured configuration.
        // Their result channel cannot populate the newly selected project.
        self.generation += 1;
        self.job.take();
        self.operation_target = None;
        let work = self
            .runtime
            .spawn(async move { service.activate_project(id).await });
        self._startup = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.initializing = false;
                match result {
                    Ok(Ok(config)) => this.activate_project(config, window, cx),
                    result => {
                        this.status = match result {
                            Ok(Err(e)) => {
                                crate::tr!("Cannot switch projects: {e}", e = format!("{e:#}"))
                            }
                            Err(e) => crate::tr!("Cannot switch projects: {e}", e = e),
                            _ => unreachable!(),
                        };
                        this.logs_open = true;
                        this.log(&this.status.clone());
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(super) fn activate_project(
        &mut self,
        config: Config,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_navigation();
        self.confirm = false;
        self.risk = None;
        self.risk_baseline = None;
        self.risk_axis = None;
        self.risk_file = None;
        self.risk_selected_file = None;
        self.queue.clear();
        self.worktrees.clear();
        self.selected_mr = None;
        self.detail = None;
        self.review = None;
        self.review_level = None;
        self.stats = None;
        self.compliance = None;
        self.insights = None;
        self.tour = None;
        self.visual = None;
        self.presentation_revision += 1;
        self.filter = Default::default();
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        self.set_queue_page(0);
        self.open_dialog = false;
        self.opening = false;
        self.context_open = None;
        self.reader_focused = false;
        self.image_viewer.update(cx, |v, cx| v.close(window, cx));
        self.threads.update(cx, |t, cx| t.set_visible(false, cx));
        self.project_threads
            .insert(self.config.namespace(), self.threads.clone());
        self.threads = self
            .project_threads
            .get(&config.namespace())
            .cloned()
            .unwrap_or_else(|| {
                let threads = cx.new(|cx| {
                    crate::threads::Threads::new(
                        self.runtime.clone(),
                        self.executor.clone(),
                        config.clone(),
                        cx,
                    )
                });
                threads.update(cx, |t, cx| t.configure(config.clone(), cx));
                threads
            });
        crate::images::clear_cache(cx);
        cx.set_http_client(Arc::new(crate::images::ImageHttpClient::new(
            self.runtime.clone(),
            self.executor.clone(),
            config.clone(),
        )));
        self.project_inputs(&config, window, cx);
        self.editing_project = None;
        self.feedback.clear();
        self.logs.clear();
        self.logs_open = false;
        self.settings = config.gitlab_url.is_empty() || !config.provider.available();
        self.threads_view = false;
        self.worktrees_view = false;
        self.status = crate::tr!("{value0} selected", value0 = config.name);
        self.language_draft = config.languages;
        gpui_component::set_locale(config.languages.interface.code());
        self.config = config;
        self.last_refresh = Instant::now() - Duration::from_secs(86400);
        cx.notify();
    }
    pub(super) fn provider_control(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .gap_2()
            .children([Provider::Gitlab, Provider::Github].map(|provider| {
                Button::new(provider.label())
                    .label(crate::i18n::t(provider.label()))
                    .when(self.draft_provider == provider, |b| b.bg(rgb(SELECTED)))
                    .disabled(self.is_busy())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.draft_provider != provider {
                            this.draft_provider = provider;
                            for input in [&this.token, &this.cf_id, &this.cf_secret] {
                                input.update(cx, |s, cx| s.set_value("", window, cx));
                            }
                            this.url.update(cx, |s, cx| {
                                s.set_value(
                                    if provider == Provider::Github {
                                        "https://github.com"
                                    } else {
                                        "https://gitlab.com"
                                    },
                                    window,
                                    cx,
                                )
                            });
                            this.feedback.clear();
                        }
                        cx.notify();
                    }))
            }))
            .into_any_element()
    }
}

//! Persistent resolver sessions and embedded interactive Ghostty terminals.
use crate::{backend::Executor, theme::*};
use futures_util::FutureExt;
use gpui::{prelude::*, *};
use gpui_component::{Disableable, Sizable, button::*};
use mrde_core::{config::Config, models::MrDetail, resolver::*};
use mrde_resolver::Update;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, mpsc},
    time::Duration,
};

pub(crate) enum Message {
    Update(Update),
    Loaded(u64, anyhow::Result<Vec<ResolverThread>>),
    Error(String, u64, String),
    TerminalReady(String, Option<String>),
    StorageError(String),
}

pub struct Threads {
    runtime: Arc<tokio::runtime::Runtime>,
    executor: Arc<dyn Executor>,
    config: Config,
    pub(crate) records: Vec<ResolverThread>,
    jobs: HashMap<String, tokio::task::JoinHandle<()>>,
    stopping: HashSet<String>,
    selected: Option<String>,
    show_closed: bool,
    session: Option<String>,
    pub(crate) sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
    load_generation: u64,
    status: String,
    loading: bool,
    #[cfg(target_os = "macos")]
    terminals: HashMap<(String, String), Entity<crate::terminal::Terminal>>,
    pending_terminal: Option<(String, String)>,
    /// Commands starting a new Claude session, by session ID; later openings resume it.
    first_launch: HashMap<String, String>,
    /// Commands the tests would have run; tests never spawn native terminals.
    #[cfg(test)]
    pub(crate) launched: Vec<String>,
    visible: bool,
    preview_terminal: bool,
    _poll: Task<()>,
}

impl Threads {
    pub fn new(
        runtime: Arc<tokio::runtime::Runtime>,
        executor: Arc<dyn Executor>,
        config: Config,
        cx: &mut Context<Self>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let poll = cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(100)).await;
                if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                    break;
                }
            }
        });
        let mut this = Self {
            runtime,
            executor,
            config: Config::default(),
            records: vec![],
            jobs: HashMap::new(),
            stopping: HashSet::new(),
            selected: None,
            show_closed: false,
            session: None,
            sender,
            receiver,
            load_generation: 0,
            status: String::new(),
            loading: false,
            #[cfg(target_os = "macos")]
            terminals: HashMap::new(),
            pending_terminal: None,
            first_launch: HashMap::new(),
            #[cfg(test)]
            launched: Vec::new(),
            visible: false,
            preview_terminal: false,
            _poll: poll,
        };
        this.configure(config, cx);
        this
    }
    pub(crate) fn preview(&mut self, interactive: bool, cx: &mut Context<Self>) {
        let mut thread = mrde_resolver::new_thread(
            crate::demo::INSTANCE.into(),
            mrde_core::models::MrId {
                project: 1,
                iid: 231,
            },
            "Handle empty daily summaries".into(),
            "a".repeat(40),
            "{}".into(),
        );
        thread.attempts = 3;
        thread.worktree = Some(std::env::temp_dir());
        thread.block("Should an empty daily summary be skipped or stored with zero entries?");
        let session = mrde_resolver::Uuid::new_v4().to_string();
        thread.sessions.push(ClaudeSession { id: session.clone(), purpose: "Fix the failed unit tests".into(),
            started_at: chrono::Utc::now(), finished_at: Some(chrono::Utc::now()),
            output: "Inspected the failing assertion.\nThe tests expect different behavior for empty summaries.\nNeed your decision before changing the implementation.\n".into() });
        self.selected = Some(thread.id.clone());
        self.session = Some(session.clone());
        if interactive {
            self.pending_terminal = Some((thread.id.clone(), session));
        }
        // Any terminal opened in the preview runs the local shell, never Claude Code.
        self.preview_terminal = true;
        self.records = vec![thread];
        self.status = "Preview · synthetic thread · no GitLab or Claude calls".into();
        self.visible = true;
        cx.notify();
    }
    pub(crate) fn preview_closed(&mut self, cx: &mut Context<Self>) {
        if let Some(thread) = self.records.first_mut() {
            thread.close();
        }
        self.show_closed = true;
        self.status = "Preview · closed thread history · no processes started".into();
        cx.notify();
    }
    /// `--ui-preview`: terminals run a harmless local shell, never Claude Code.
    pub(crate) fn use_preview_terminal(&mut self) {
        self.preview_terminal = true;
    }
    pub fn configure(&mut self, config: Config, cx: &mut Context<Self>) {
        // A new connection must not leave automation using the previous credentials.
        for id in self.jobs.keys().cloned().collect::<Vec<_>>() {
            self.stop(&id, None, cx);
        }
        self.config = config;
        self.load_generation += 1;
        self.records.clear();
        self.show_closed = false;
        self.selected = None;
        self.session = None;
        self.pending_terminal = None;
        self.loading = false;
        self.status.clear();
        #[cfg(target_os = "macos")]
        self.terminals.clear();
        if self.config.gitlab_url.is_empty() || !self.config.provider.available() {
            return;
        }
        self.loading = true;
        let generation = self.load_generation;
        let sender = self.sender.clone();
        let executor = self.executor.clone();
        let config = self.config.clone();
        self.runtime.spawn(async move {
            let result = executor.threads(config).await;
            let _ = sender.send(Message::Loaded(generation, result));
        });
        cx.notify();
    }
    fn save(&self, thread: ResolverThread) {
        let executor = self.executor.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            if let Err(error) = executor.save_thread(thread).await {
                let _ = sender.send(Message::StorageError(crate::tr!(
                    "Cannot save thread: {error}",
                    error = format!("{error:#}")
                )));
            }
        });
    }
    pub fn start(&mut self, detail: &MrDetail, cx: &mut Context<Self>) {
        self.show_closed = false;
        if let Some(existing) = self.records.iter().find(|t| {
            t.closed_at.is_none()
                && t.automated()
                && t.mr == detail.mr.id
                && (t.state.active() || self.terminal_alive(&t.id, cx))
        }) {
            self.selected = Some(existing.id.clone());
            self.session = None;
            cx.notify();
            return;
        }
        let context = serde_json::json!({"title": detail.mr.title, "description": detail.mr.description, "issues": detail.issues});
        let thread = mrde_resolver::new_thread(
            self.config.namespace(),
            detail.mr.id.clone(),
            detail.mr.title.clone(),
            detail.mr.sha.clone(),
            context.to_string(),
        );
        self.selected = Some(thread.id.clone());
        self.session = None;
        self.records.insert(0, thread.clone());
        self.launch(thread, cx);
    }
    /// Opens Claude Code read-only on the merge request, in a questions thread: no
    /// automation, rebase, or push. The same commit reuses its thread and session.
    pub fn ask(&mut self, ready: crate::backend::AskReady, cx: &mut Context<Self>) {
        self.show_closed = false;
        let existing = self.records.iter().find(|t| {
            t.closed_at.is_none()
                && t.kind == ThreadKind::Questions
                && t.mr == ready.mr
                && t.remote_sha == ready.sha
                && t.worktree.as_ref() == Some(&ready.worktree)
        });
        let (id, session) = match existing.and_then(|t| Some((t, t.sessions.last()?))) {
            Some((thread, session)) => {
                self.status = "Back to your questions about this merge request.".into();
                (thread.id.clone(), session.id.clone())
            }
            None => {
                let mut thread = mrde_resolver::new_thread(
                    self.config.namespace(),
                    ready.mr,
                    ready.title,
                    ready.sha,
                    ready.context,
                );
                thread.kind = ThreadKind::Questions;
                thread.state = ResolverState::Stopped;
                thread.worktree = Some(ready.worktree);
                let session = mrde_resolver::Uuid::new_v4().to_string();
                thread.sessions.push(ClaudeSession {
                    id: session.clone(),
                    purpose: "Questions about the merge request".into(),
                    started_at: chrono::Utc::now(),
                    finished_at: None,
                    output: String::new(),
                });
                thread.event(
                    "Claude Code opened read-only (plan mode) with the merge request context.",
                );
                self.first_launch.insert(
                    session.clone(),
                    crate::terminal::ask_command(
                        &self.config.claude_path,
                        &session,
                        &ready.context_dir,
                        &ready.prompt,
                    ),
                );
                let id = thread.id.clone();
                self.records.insert(0, thread.clone());
                self.save(thread);
                self.status =
                    "Claude Code is reading the merge request. Ask your questions below.".into();
                (id, session)
            }
        };
        self.selected = Some(id.clone());
        self.session = Some(session.clone());
        self.pending_terminal = Some((id, session));
        cx.notify();
    }
    fn launch(&mut self, thread: ResolverThread, cx: &mut Context<Self>) {
        let id = thread.id.clone();
        let generation = thread.generation;
        let sender = self.sender.clone();
        let output = sender.clone();
        let executor = self.executor.clone();
        let config = self.config.clone();
        let emit = Arc::new(move |update| {
            let _ = output.send(Message::Update(update));
        });
        let key = id.clone();
        self.jobs.insert(
            id,
            self.runtime.spawn(async move {
                let result = std::panic::AssertUnwindSafe(executor.resolve(config, thread, emit))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("Resolver stopped unexpectedly")));
                if let Err(error) = result {
                    let _ = sender.send(Message::Error(key, generation, format!("{error:#}")));
                }
            }),
        );
        self.status = "Super Resolver is running in the background.".into();
        cx.notify();
    }
    fn stop(&mut self, id: &str, terminal: Option<String>, cx: &mut Context<Self>) {
        if let Some(thread) = self.records.iter_mut().find(|t| t.id == id) {
            thread.generation += 1;
            thread.sequence = 0;
            thread.state = ResolverState::Stopped;
            thread.event("Stopped. The worktree and Claude sessions were preserved.");
            let thread = thread.clone();
            self.save(thread);
        }
        self.status = "Stopping automation; the worktree and sessions are preserved.".into();
        self.stopping.insert(id.to_string());
        let job = self.jobs.remove(id);
        if let Some(job) = &job {
            job.abort();
        }
        // Wait for the non-interactive child to stop before resuming the same session.
        let sender = self.sender.clone();
        let id = id.to_string();
        self.runtime.spawn(async move {
            if let Some(job) = job {
                let _ = job.await;
            }
            let _ = sender.send(Message::TerminalReady(id, terminal));
        });
        cx.notify();
    }
    fn close_thread(&mut self, id: &str, cx: &mut Context<Self>) {
        // Stop/await the worker first; the newer generation rejects queued late writes.
        self.stop(id, None, cx);
        if let Some(thread) = self.records.iter_mut().find(|t| t.id == id) {
            thread.close();
            let thread = thread.clone();
            self.save(thread);
        }
        if self
            .pending_terminal
            .as_ref()
            .is_some_and(|(thread, _)| thread == id)
        {
            self.pending_terminal = None;
        }
        #[cfg(target_os = "macos")]
        {
            let keys: Vec<_> = self
                .terminals
                .keys()
                .filter(|(thread, _)| thread == id)
                .cloned()
                .collect();
            for key in keys {
                if let Some(terminal) = self.terminals.remove(&key) {
                    terminal.update(cx, |terminal, _| terminal.set_visible(false));
                }
            }
        }
        self.selected = self
            .records
            .iter()
            .find(|t| t.closed_at.is_none())
            .map(|t| t.id.clone());
        self.session = None;
        self.status = "Thread closed. Find its history under Closed threads.".into();
        cx.notify();
    }
    fn reopen_thread(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.stopping.contains(id) {
            return;
        }
        if let Some(thread) = self.records.iter_mut().find(|t| t.id == id) {
            thread.reopen();
            let thread = thread.clone();
            self.save(thread);
        }
        self.show_closed = false;
        self.selected = Some(id.into());
        self.session = None;
        self.status = "Thread reopened. No Claude process was started.".into();
        cx.notify();
    }
    fn resume(&mut self, id: &str, cx: &mut Context<Self>) {
        if self
            .records
            .iter()
            .any(|t| t.id == id && t.closed_at.is_some())
        {
            return;
        }
        if self.stopping.contains(id) {
            return;
        }
        if self.terminal_alive(id, cx) {
            self.status = "Exit the interactive Claude session before resuming automation.".into();
            cx.notify();
            return;
        }
        if let Some(thread) = self
            .records
            .iter_mut()
            .find(|t| t.id == id && t.automated() && !t.state.active())
        {
            thread.resume();
            let thread = thread.clone();
            self.launch(thread, cx);
        }
    }
    fn terminal_alive(&self, id: &str, cx: &App) -> bool {
        #[cfg(target_os = "macos")]
        return self
            .terminals
            .iter()
            .any(|((thread, _), terminal)| thread == id && terminal.read(cx).is_alive());
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (id, cx);
            false
        }
    }
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.visible = visible;
        #[cfg(target_os = "macos")]
        for ((thread, session), terminal) in &self.terminals {
            let show = visible
                && self.selected.as_ref() == Some(thread)
                && self.session.as_ref() == Some(session);
            terminal.update(cx, |terminal, _| terminal.set_visible(show));
        }
    }
    pub(crate) fn poll(&mut self, cx: &mut Context<Self>) {
        // Bound per-frame work even when Claude emits many tool events.
        for _ in 0..64 {
            let Ok(message) = self.receiver.try_recv() else {
                break;
            };
            match message {
                Message::Loaded(generation, result) if generation == self.load_generation => {
                    self.loading = false;
                    match result {
                        Ok(records) => {
                            for mut thread in records {
                                if self.records.iter().any(|t| t.id == thread.id) {
                                    continue;
                                }
                                if thread.closed_at.is_none() && thread.state.active() {
                                    thread.generation += 1;
                                    thread.sequence = 0;
                                    thread.state = ResolverState::Interrupted;
                                    thread.event("The previous run was interrupted. Inspect the session before resuming.");
                                    self.save(thread.clone());
                                }
                                self.records.push(thread);
                            }
                        }
                        Err(error) => {
                            self.status = crate::tr!(
                                "Cannot load Threads: {error}",
                                error = format!("{error:#}")
                            )
                        }
                    }
                }
                Message::Loaded(..) => {}
                Message::Update(Update::Thread(thread)) => {
                    if let Some(current) = self.records.iter_mut().find(|t| {
                        t.id == thread.id
                            && t.generation == thread.generation
                            && t.sequence <= thread.sequence
                    }) {
                        if !thread.state.active() {
                            self.jobs.remove(&thread.id);
                            self.status = format!("{} · {}", thread.title, thread.state.label());
                        }
                        // Preserve streamed text if the state snapshot precedes its completion.
                        let mut thread = *thread;
                        for session in &mut thread.sessions {
                            if session.output.is_empty()
                                && let Some(old) =
                                    current.sessions.iter().find(|s| s.id == session.id)
                            {
                                session.output = old.output.clone();
                            }
                        }
                        *current = thread;
                    }
                }
                Message::Update(Update::Output {
                    thread,
                    generation,
                    session,
                    text,
                }) => {
                    if let Some(record) = self
                        .records
                        .iter_mut()
                        .find(|t| t.id == thread && t.generation == generation)
                        && let Some(session) = record.sessions.iter_mut().find(|s| s.id == session)
                    {
                        mrde_claude::resolver::append_output(&mut session.output, &text);
                    }
                }
                Message::Error(id, generation, error) => {
                    if let Some(thread) = self
                        .records
                        .iter_mut()
                        .find(|t| t.id == id && t.generation == generation)
                    {
                        self.jobs.remove(&id);
                        thread.block(error);
                        self.status =
                            "Super Resolver needs your input. Select the thread for details."
                                .into();
                        let thread = thread.clone();
                        self.save(thread);
                    }
                }
                Message::TerminalReady(id, session) => {
                    self.stopping.remove(&id);
                    self.status = if self
                        .records
                        .iter()
                        .any(|t| t.id == id && t.closed_at.is_some())
                    {
                        "Thread closed. Find its history under Closed threads.".into()
                    } else {
                        "Automation stopped. Open a session or Resume when ready.".into()
                    };
                    if let Some(session) = session
                        && self.selected.as_ref() == Some(&id)
                        && self
                            .records
                            .iter()
                            .any(|t| t.id == id && t.closed_at.is_none() && !t.state.active())
                    {
                        self.pending_terminal = Some((id, session));
                    }
                }
                Message::StorageError(error) => self.status = error,
            }
            cx.notify();
        }
    }
    #[cfg(target_os = "macos")]
    fn open_terminal(
        &mut self,
        id: String,
        session: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .terminals
            .get(&(id.clone(), session.clone()))
            .is_some_and(|t| t.read(cx).is_alive())
        {
            if let Some(terminal) = self.terminals.get(&(id.clone(), session.clone())) {
                terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
            }
            self.selected = Some(id);
            self.session = Some(session);
            return;
        }
        if self.terminal_alive(&id, cx) {
            self.status =
                "Close the other interactive session for this thread before opening another."
                    .into();
            return;
        }
        let Some(directory) = self
            .records
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.worktree.clone())
        else {
            self.status = "The worktree has not been created yet.".into();
            return;
        };
        if mrde_resolver::Uuid::parse_str(&session).is_err() {
            self.status = "Invalid Claude session ID.".into();
            return;
        }
        let read_only = self
            .records
            .iter()
            .any(|t| t.id == id && t.kind == ThreadKind::Questions);
        let first = self.first_launch.remove(&session);
        let command = if self.preview_terminal {
            // Preview-only local shell: no Claude, network, credentials or Git writes.
            crate::terminal::preview_command()
        } else if let Some(command) = &first {
            command.clone()
        } else {
            crate::terminal::resume_command(&self.config.claude_path, &session, read_only)
        };
        self.spawn_terminal(id, session, command, directory, first, window, cx);
    }
    /// Tests record the command instead of attaching a native surface to a test window.
    #[cfg(all(target_os = "macos", test))]
    #[allow(clippy::too_many_arguments)]
    fn spawn_terminal(
        &mut self,
        id: String,
        session: String,
        command: String,
        _directory: std::path::PathBuf,
        _first: Option<String>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.launched.push(command);
        self.selected = Some(id);
        self.session = Some(session);
    }
    #[cfg(all(target_os = "macos", not(test)))]
    #[allow(clippy::too_many_arguments)]
    fn spawn_terminal(
        &mut self,
        id: String,
        session: String,
        command: String,
        directory: std::path::PathBuf,
        first: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut options = mrde_ghostty_adapter::TerminalOptions::new(command, directory);
        options.quiet_login = true;
        options.configuration = crate::terminal::configuration();
        match crate::terminal::Terminal::spawn(options, window, cx) {
            Ok(terminal) => {
                self.selected = Some(id.clone());
                self.session = Some(session.clone());
                self.terminals.insert((id, session), terminal);
                // Exercise real GPUI key dispatch into the native PTY in preview only.
                if self.preview_terminal && std::env::var_os("MRDE_TERMINAL_INPUT").is_some() {
                    cx.spawn_in(window, async move |_, cx| {
                        Timer::after(Duration::from_millis(900)).await;
                        let _ = cx.update(|window, cx| {
                            for key in ["o", "k", "enter"] {
                                window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
                            }
                        });
                    })
                    .detach();
                }
            }
            Err(error) => {
                // The session was never started: the next opening must start it again.
                if let Some(first) = first {
                    self.first_launch.insert(session, first);
                }
                self.status = crate::tr!("Cannot open the terminal: {error}", error = error);
            }
        }
    }
}
impl Drop for Threads {
    fn drop(&mut self) {
        for (_, job) in self.jobs.drain() {
            job.abort();
        }
    }
}

impl Render for Threads {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::i18n::set(self.config.languages.interface);
        #[cfg(target_os = "macos")]
        if let Some((id, session)) = self.pending_terminal.take() {
            self.open_terminal(id, session, window, cx);
        }
        self.set_visible(self.visible, cx);
        let mut list = div()
            .id("thread-list")
            .debug_selector(|| "thread-list".into())
            .w(px(280.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(rgb(PANEL_LINE))
            .bg(rgb(SIDEBAR))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(16.))
                    .text_color(rgb(HEADING))
                    .mb_3()
                    .child(crate::i18n::t("Threads")),
            );
        let closed_count = self
            .records
            .iter()
            .filter(|t| t.closed_at.is_some())
            .count();
        list = list.child(
            div()
                .debug_selector(|| "toggle-closed-threads".into())
                .child(
                    Button::new("toggle-closed-threads")
                        .small()
                        .ghost()
                        .label(if self.show_closed {
                            "Open threads".to_string()
                        } else {
                            crate::tr!(
                                "Closed threads ({closed_count})",
                                closed_count = closed_count
                            )
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_closed = !this.show_closed;
                            this.selected = None;
                            this.session = None;
                            this.pending_terminal = None;
                            cx.notify();
                        })),
                ),
        );
        if !self
            .records
            .iter()
            .any(|t| t.closed_at.is_some() == self.show_closed)
        {
            list = list.child(if self.loading {
                "Loading threads…"
            } else if self.show_closed {
                "No closed threads."
            } else {
                "Ask Claude or start Super Resolver from a merge request."
            });
        }
        for thread in self
            .records
            .iter()
            .filter(|t| t.closed_at.is_some() == self.show_closed)
        {
            let id = thread.id.clone();
            let selected = self.selected.as_ref() == Some(&id);
            list = list.child(
                div()
                    .id(SharedString::from(format!("thread-{id}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("thread-{id}")
                    })
                    .p_2()
                    .rounded(px(6.))
                    .bg(rgb(if selected { SELECTED } else { SIDEBAR }))
                    .cursor_pointer()
                    .child(div().text_size(px(13.)).child(thread.title.clone()))
                    .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child(
                        if thread.automated() {
                            format!(
                                "!{} · {} · {}/3",
                                thread.mr.iid,
                                thread.label(),
                                thread.attempts
                            )
                        } else {
                            crate::tr!(
                                "!{value0} · {value1} · read-only",
                                value0 = thread.mr.iid,
                                value1 = thread.label()
                            )
                        },
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(id.clone());
                        this.session = None;
                        cx.notify();
                    })),
            );
            for (index, session) in thread.sessions.iter().enumerate() {
                let id = thread.id.clone();
                let session_id = session.id.clone();
                list = list.child(
                    div()
                        .id(SharedString::from(format!("session-{session_id}")))
                        .pl_4()
                        .py_2()
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(rgb(if self.session.as_ref() == Some(&session_id) {
                            HEADING
                        } else {
                            MUTED
                        }))
                        .child(crate::tr!("Claude · session {value0}", value0 = index + 1))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = Some(id.clone());
                            this.session = Some(session_id.clone());
                            cx.notify();
                        })),
                );
            }
        }
        let mut reader = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(READING));
        if let Some(thread) = self
            .records
            .iter()
            .find(|t| Some(&t.id) == self.selected.as_ref())
        {
            let id = thread.id.clone();
            let active = thread.state.active();
            let session = self
                .session
                .as_ref()
                .and_then(|id| thread.sessions.iter().find(|s| &s.id == id))
                .or_else(|| thread.sessions.last());
            let mut toolbar = div()
                .flex_shrink_0()
                .p_3()
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(rgb(PANEL_LINE))
                .child(div().flex_1().child(crate::i18n::t(thread.label())));
            if thread.closed_at.is_some() {
                let id = id.clone();
                toolbar = toolbar.child(
                    div().debug_selector(|| "reopen-thread".into()).child(
                        Button::new("reopen-thread")
                            .small()
                            .label(crate::i18n::t("Reopen thread"))
                            .disabled(self.stopping.contains(&id))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.reopen_thread(&id, cx)),
                            ),
                    ),
                );
            } else if active {
                let id = id.clone();
                toolbar = toolbar.child(
                    div().debug_selector(|| "stop-resolver".into()).child(
                        Button::new("stop-resolver")
                            .small()
                            .label(crate::i18n::t("Stop"))
                            .on_click(cx.listener(move |this, _, _, cx| this.stop(&id, None, cx))),
                    ),
                );
            } else if thread.automated() {
                let id = id.clone();
                toolbar = toolbar.child(
                    div().debug_selector(|| "resume-resolver".into()).child(
                        Button::new("resume-resolver")
                            .small()
                            .label(crate::i18n::t("Resume · 3 attempts"))
                            .disabled(self.terminal_alive(&id, cx) || self.stopping.contains(&id))
                            .on_click(cx.listener(move |this, _, _, cx| this.resume(&id, cx))),
                    ),
                );
            }
            if let Some(session) = session.filter(|_| thread.closed_at.is_none()) {
                let id = id.clone();
                let sid = session.id.clone();
                toolbar = toolbar.child(
                    Button::new("thread-terminal")
                        .small()
                        .label(if active {
                            crate::i18n::t("Take over")
                        } else {
                            crate::i18n::t("Open terminal")
                        })
                        .disabled(self.stopping.contains(&id))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if active {
                                this.stop(&id, Some(sid.clone()), cx);
                            } else {
                                this.pending_terminal = Some((id.clone(), sid.clone()));
                                cx.notify();
                            }
                        })),
                );
            }
            #[cfg(target_os = "macos")]
            if let Some(session) = self
                .session
                .as_ref()
                .filter(|s| self.terminals.contains_key(&(id.clone(), (*s).clone())))
            {
                let key = (id.clone(), session.clone());
                toolbar = toolbar.child(
                    Button::new("close-terminal")
                        .small()
                        .label(crate::i18n::t("Close terminal"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(terminal) = this.terminals.remove(&key) {
                                terminal.update(cx, |terminal, _| terminal.set_visible(false));
                            }
                            this.status =
                                "Terminal closed. The Claude session can be reopened.".into();
                            cx.notify();
                        })),
                );
            }
            if thread.closed_at.is_none() {
                let id = id.clone();
                toolbar = toolbar.child(div().debug_selector(|| "close-thread".into()).child(
                    Button::new("close-thread").small().label(crate::i18n::t("Close thread"))
                        .tooltip(crate::i18n::t("Stop activity and move this thread to Closed threads; preserve its worktree and history."))
                        .on_click(cx.listener(move |this, _, _, cx| this.close_thread(&id, cx)))));
            }
            reader = reader.child(toolbar.flex_wrap());
            if let Some(question) = &thread.question {
                reader = reader.child(div().p_3().text_color(rgb(WARNING)).child(question.clone()));
            }
            #[cfg(target_os = "macos")]
            let terminal = self
                .session
                .as_ref()
                .and_then(|session| self.terminals.get(&(id.clone(), session.clone())))
                .cloned();
            #[cfg(not(target_os = "macos"))]
            let terminal: Option<Entity<crate::image_viewer::ImageViewer>> = None;
            if let Some(terminal) = terminal {
                reader = reader.child(div().flex_1().min_h_0().child(terminal));
            } else {
                // Questions live in Claude Code's own session; the thread keeps its events.
                let text = if let Some(session) =
                    session.filter(|s| thread.automated() && self.session.as_ref() == Some(&s.id))
                {
                    session.output.clone()
                } else {
                    thread
                        .events
                        .iter()
                        .map(|event| {
                            format!("{}  {}\n", event.at.format("%H:%M:%S"), event.message)
                        })
                        .collect()
                };
                reader = reader.child(
                    div()
                        .id("thread-output")
                        .debug_selector(|| "thread-output".into())
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .p_4()
                        .text_size(px(13.))
                        .line_height(px(20.))
                        .font_family(CODE_FONT)
                        .child(if text.is_empty() {
                            "Waiting for Claude output…".into()
                        } else {
                            text
                        }),
                );
            }
        } else {
            reader = reader.child(div().p_5().text_color(rgb(MUTED)).child(crate::i18n::t(
                "Select a thread to inspect its sessions and pipeline history.",
            )));
        }
        div()
            .debug_selector(|| "threads-view".into())
            .size_full()
            .flex()
            .text_color(rgb(INK))
            .child(list)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(reader)
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(self.status.clone()),
                    ),
            )
    }
}

//! Async adapter for terminal effects. The input/render thread never performs forge,
//! Keychain, Git, or Claude operations.
use crate::{
    app::{App, Effect},
    ui::View,
};
use anyhow::Context;
use mrde_core::language::Languages;
use mrde_service::{
    backend::{Executor, Outcome},
    settings::ConnectionSettings,
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};
use std::{
    collections::HashMap,
    io::{self, IsTerminal},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

enum Reply {
    Outcome(u64, Box<anyhow::Result<Outcome>>),
    Languages(u64, Languages, anyhow::Result<()>),
}
pub struct Driver {
    runtime: tokio::runtime::Runtime,
    executor: Arc<dyn Executor>,
    settings: Arc<dyn ConnectionSettings>,
    sender: mpsc::Sender<Reply>,
    receiver: mpsc::Receiver<Reply>,
    jobs: HashMap<u64, tokio::task::JoinHandle<()>>,
}
impl Driver {
    pub fn new(
        executor: Arc<dyn Executor>,
        settings: Arc<dyn ConnectionSettings>,
    ) -> anyhow::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        Ok(Self {
            runtime: tokio::runtime::Runtime::new()?,
            executor,
            settings,
            sender,
            receiver,
            jobs: HashMap::new(),
        })
    }
    /// Returns terminal-only effects. Every service call is scheduled off the UI thread.
    pub fn submit(&mut self, effects: Vec<Effect>) -> Vec<Effect> {
        let mut terminal = vec![];
        for effect in effects {
            let sender = self.sender.clone();
            let executor = self.executor.clone();
            let settings = self.settings.clone();
            let (token, task) = match effect {
                Effect::Run {
                    token,
                    config,
                    action,
                } => (
                    token,
                    self.runtime.spawn(async move {
                        let result = executor.execute(*config, action).await;
                        let _ = sender.send(Reply::Outcome(token, Box::new(result)));
                    }),
                ),
                Effect::Activate { token, id } => (
                    token,
                    self.runtime.spawn(async move {
                        let result = settings
                            .activate_project(id)
                            .await
                            .map(Outcome::SettingsSaved);
                        let _ = sender.send(Reply::Outcome(token, Box::new(result)));
                    }),
                ),
                Effect::SaveLanguages { token, languages } => (
                    token,
                    self.runtime.spawn(async move {
                        let result = settings.save_languages(languages).await;
                        let _ = sender.send(Reply::Languages(token, languages, result));
                    }),
                ),
                Effect::SaveQueue { token, config } => (
                    token,
                    self.runtime.spawn(async move {
                        let roles = config.mr_roles.clone();
                        let sort = config.mr_sort;
                        let result = settings
                            .save_queue_preferences(*config, roles, sort)
                            .await
                            .map(|config| Outcome::QueuePreferencesSaved(config, vec![]));
                        let _ = sender.send(Reply::Outcome(token, Box::new(result)));
                    }),
                ),
                Effect::Cancel(token) => {
                    if let Some(task) = self.jobs.remove(&token) {
                        task.abort();
                    }
                    continue;
                }
                other => {
                    terminal.push(other);
                    continue;
                }
            };
            self.jobs.insert(token, task);
        }
        terminal
    }
    pub fn poll(&mut self, app: &mut App) -> (bool, Vec<Effect>) {
        let mut changed = false;
        let mut effects = vec![];
        while let Ok(reply) = self.receiver.try_recv() {
            changed = true;
            match reply {
                Reply::Outcome(token, result) => {
                    self.jobs.remove(&token);
                    effects.extend(app.complete(token, *result));
                }
                Reply::Languages(token, languages, result) => {
                    self.jobs.remove(&token);
                    app.languages_saved(token, languages, result);
                }
            }
        }
        (changed, self.submit(effects))
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        for job in self.jobs.values() {
            job.abort();
        }
    }
}

struct ScreenGuard;
impl ScreenGuard {
    fn enter() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        Ok(Self)
    }
    fn suspend(&self) -> anyhow::Result<()> {
        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        Ok(())
    }
    fn resume(&self) -> anyhow::Result<()> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        Ok(())
    }
}
impl Drop for ScreenGuard {
    fn drop(&mut self) {
        let _ = self.suspend();
    }
}

pub fn run(mut app: App, mut driver: Driver) -> anyhow::Result<()> {
    anyhow::ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "An interactive terminal is required. Use --demo --snapshot <file.svg> for an export."
    );
    let guard = ScreenGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    let mut view = View::default();
    let mut dirty = true;
    let mut refreshed = Instant::now();
    if !app.demo && !app.config.gitlab_url.is_empty() {
        driver.submit(app.request(mrde_service::backend::Action::Refresh));
    }
    loop {
        let (changed, mut effects) = driver.poll(&mut app);
        dirty |= changed;
        if dirty {
            terminal.draw(|frame| view.draw(frame, &mut app))?;
            dirty = false;
        }
        if event::poll(Duration::from_millis(50))? {
            let actions = match event::read()? {
                Event::Key(key) => app.key(key),
                Event::Mouse(mouse) => app.mouse(mouse),
                Event::Resize(_, _) => vec![],
                _ => vec![],
            };
            dirty = true;
            effects.extend(driver.submit(actions));
        }
        if !app.demo
            && !app.config.gitlab_url.is_empty()
            && refreshed.elapsed() >= Duration::from_secs(app.config.refresh_seconds.max(30))
        {
            driver.submit(app.request(mrde_service::backend::Action::Refresh));
            refreshed = Instant::now();
            dirty = true;
        }
        for effect in effects {
            match effect {
                Effect::Quit => return Ok(()),
                Effect::Chat(ready) => {
                    guard.suspend()?;
                    let result = std::process::Command::new(&app.config.claude_path)
                        .current_dir(&ready.worktree)
                        .args(["--permission-mode", "plan", "--add-dir"])
                        .arg(&ready.context_dir)
                        .arg(&ready.prompt)
                        .status()
                        .context("Cannot launch Claude Code");
                    guard.resume()?;
                    terminal.clear()?;
                    match result {
                        Ok(status) if status.success() => {
                            app.status = "Returned from Claude".into();
                            app.error = false;
                        }
                        Ok(status) => {
                            app.status = format!("Claude exited with {status}");
                            app.error = true;
                        }
                        Err(error) => {
                            app.status = format!("{error:#}");
                            app.error = true;
                        }
                    }
                    dirty = true;
                }
                _ => unreachable!("services run on the worker runtime"),
            }
        }
    }
}

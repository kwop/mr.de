//! Terminal application state. Effects are executed by the runtime, never by rendering.
use mrde_core::{
    config::{Config, QueueSort},
    language::{Language, Languages, translate},
    models::*,
    risk::*,
};
use mrde_service::{
    backend::{Action, AskReady, Outcome},
    projects::Projects,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Inbox,
    Reader,
    Projects,
    Settings,
    Worktrees,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Description,
    Files,
    Radar,
    Review,
    History,
}
impl Tab {
    pub const ALL: [Self; 5] = [
        Self::Description,
        Self::Files,
        Self::Radar,
        Self::Review,
        Self::History,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Description => "Description",
            Self::Files => "Files",
            Self::Radar => "Risk radar",
            Self::Review => "Pre-review",
            Self::History => "History",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lane {
    Refresh,
    Navigation,
    Operation,
    Preferences,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Navigation {
    Inbox,
    Projects,
    Worktrees,
    Settings,
}
#[derive(Clone, Debug)]
pub enum Hit {
    Nav(Navigation),
    Row(usize),
    Tab(Tab),
    Axis(usize),
    Project(usize),
    Interface(Language),
    Information(Language),
    SaveLanguages,
}
pub enum Effect {
    Run {
        token: u64,
        config: Box<Config>,
        action: Action,
    },
    Activate {
        token: u64,
        id: String,
    },
    SaveLanguages {
        token: u64,
        languages: Languages,
    },
    SaveQueue {
        token: u64,
        config: Box<Config>,
    },
    Cancel(u64),
    Chat(Box<AskReady>),
    Quit,
}
struct Job {
    token: u64,
    label: &'static str,
    target: Option<(MrId, String)>,
    cancellable: bool,
}
pub enum Confirmation {
    Approve(Box<MrDetail>),
    Tools(Box<MrDetail>),
    Lint(Box<MrDetail>, String),
}

pub struct App {
    pub catalog: Projects,
    pub config: Config,
    pub demo: bool,
    pub screen: Screen,
    pub tab: Tab,
    pub queue: Vec<MergeRequest>,
    pub selected: usize,
    pub project_selected: usize,
    pub search: String,
    pub searching: bool,
    pub detail: Option<Box<MrDetail>>,
    pub risk: Option<Box<RiskReport>>,
    pub baseline: Option<Baseline>,
    pub review: Option<Review>,
    pub insights: Option<Box<mrde_core::insights::Insights>>,
    pub worktrees: Vec<mrde_git::WorktreeEntry>,
    pub axis: usize,
    pub scroll: u16,
    pub level: ReviewLevel,
    pub language_draft: Languages,
    pub status: String,
    pub error: bool,
    pub help: bool,
    pub confirmation: Option<Confirmation>,
    pub hits: Vec<(ratatui::layout::Rect, Hit)>,
    jobs: HashMap<Lane, Job>,
    next_token: u64,
}
impl App {
    pub fn new(catalog: Projects, queue: Vec<MergeRequest>, demo: bool) -> anyhow::Result<Self> {
        let config = catalog.current()?;
        let mut app = Self {
            language_draft: config.languages,
            catalog,
            config,
            demo,
            screen: Screen::Inbox,
            tab: Tab::Description,
            queue,
            selected: 0,
            project_selected: 0,
            search: String::new(),
            searching: false,
            detail: None,
            risk: None,
            baseline: None,
            review: None,
            insights: None,
            worktrees: vec![],
            axis: 0,
            scroll: 0,
            level: ReviewLevel::Standard,
            status: "Ready".into(),
            error: false,
            help: false,
            confirmation: None,
            hits: vec![],
            jobs: HashMap::new(),
            next_token: 0,
        };
        app.sort();
        Ok(app)
    }
    pub fn demo() -> Self {
        let config = Config {
            name: "Platform".into(),
            gitlab_url: mrde_service::demo::INSTANCE.into(),
            ..Default::default()
        };
        let mut catalog = Projects::single(config.clone());
        catalog.projects.push(Config {
            id: "community".into(),
            name: "Community".into(),
            provider: mrde_core::config::Provider::Github,
            gitlab_url: "https://github.com".into(),
            ..config
        });
        Self::new(catalog, mrde_service::demo::queue(), true).expect("valid demo")
    }
    pub fn t<'a>(&self, source: &'a str) -> &'a str {
        translate(self.config.languages.interface, source)
    }
    pub fn filtered(&self) -> Vec<&MergeRequest> {
        let filter = mrde_core::inbox::QueueFilter {
            query: self.search.clone(),
            ..Default::default()
        };
        self.queue.iter().filter(|mr| filter.matches(mr)).collect()
    }
    fn sort(&mut self) {
        mrde_core::scoring::sort_queue(&mut self.queue, self.config.mr_sort, chrono::Utc::now());
    }
    pub fn busy(&self) -> bool {
        self.jobs.contains_key(&Lane::Operation)
            || (self.jobs.contains_key(&Lane::Preferences)
                && self.language_draft != self.config.languages)
    }
    pub fn progress(&self) -> Option<&'static str> {
        [
            Lane::Operation,
            Lane::Navigation,
            Lane::Refresh,
            Lane::Preferences,
        ]
        .iter()
        .find_map(|lane| self.jobs.get(lane).map(|j| j.label))
    }
    fn begin(
        &mut self,
        lane: Lane,
        label: &'static str,
        target: Option<(MrId, String)>,
        cancellable: bool,
    ) -> (u64, Vec<Effect>) {
        let mut effects = vec![];
        if let Some(old) = self.jobs.remove(&lane) {
            effects.push(Effect::Cancel(old.token));
        }
        self.next_token += 1;
        let token = self.next_token;
        self.jobs.insert(
            lane,
            Job {
                token,
                label,
                target,
                cancellable,
            },
        );
        self.error = false;
        (token, effects)
    }
    pub fn request(&mut self, action: Action) -> Vec<Effect> {
        let (lane, label, cancellable, target) = match &action {
            Action::Refresh => (Lane::Refresh, "Refreshing merge requests…", true, None),
            Action::Select(_) | Action::Open(_) => {
                (Lane::Navigation, "Loading merge request…", true, None)
            }
            Action::Worktrees => (Lane::Navigation, "Loading worktrees…", true, None),
            Action::Approve(d) => (
                Lane::Operation,
                "Recording approval…",
                false,
                Some((d.mr.id.clone(), d.mr.sha.clone())),
            ),
            Action::Risk(d) => (
                Lane::Operation,
                "Calculating risks…",
                true,
                Some((d.mr.id.clone(), d.mr.sha.clone())),
            ),
            Action::RiskBaseline(d) => (
                Lane::Operation,
                "Updating team median…",
                true,
                Some((d.mr.id.clone(), d.mr.sha.clone())),
            ),
            Action::Review(d, _) => (
                Lane::Operation,
                "Claude is reviewing…",
                true,
                Some((d.mr.id.clone(), d.mr.sha.clone())),
            ),
            Action::Ask(d, _) | Action::AskRisk(d, ..) => (
                Lane::Operation,
                "Preparing Claude context…",
                true,
                Some((d.mr.id.clone(), d.mr.sha.clone())),
            ),
            Action::Analyze(d) => (
                Lane::Operation,
                "Analyzing worktree…",
                true,
                Some((d.mr.id.clone(), d.mr.sha.clone())),
            ),
            Action::SetupRiskTools(d) | Action::ApproveRiskCommand(d, _) => (
                Lane::Operation,
                "Preparing risk tools…",
                true,
                Some((d.mr.id.clone(), d.mr.sha.clone())),
            ),
            _ => (
                Lane::Operation,
                "Working…",
                true,
                self.detail
                    .as_ref()
                    .map(|d| (d.mr.id.clone(), d.mr.sha.clone())),
            ),
        };
        if (lane == Lane::Operation && self.busy())
            || (lane == Lane::Refresh && self.jobs.contains_key(&Lane::Refresh))
        {
            return vec![];
        }
        let (token, mut effects) = self.begin(lane, label, target, cancellable);
        effects.push(Effect::Run {
            token,
            config: Box::new(self.config.clone()),
            action,
        });
        effects
    }
    pub fn complete(&mut self, token: u64, result: anyhow::Result<Outcome>) -> Vec<Effect> {
        let Some(lane) = self
            .jobs
            .iter()
            .find(|(_, job)| job.token == token)
            .map(|(lane, _)| *lane)
        else {
            return vec![];
        };
        let job = self.jobs.remove(&lane).expect("known job");
        if job.target.as_ref().is_some_and(|(id, sha)| {
            self.detail
                .as_ref()
                .is_none_or(|d| d.mr.id != *id || d.mr.sha != *sha)
        }) {
            return vec![];
        }
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                self.error = true;
                self.status = format!("{error:#}");
                return vec![];
            }
        };
        self.error = false;
        self.status = "Ready".into();
        match outcome {
            Outcome::Queue(queue) => {
                self.queue = queue;
                self.sort();
                self.selected = self.selected.min(self.filtered().len().saturating_sub(1));
            }
            Outcome::Detail(detail, review) => self.set_detail(detail, review),
            Outcome::DetailWith(detail, review, restored) => {
                self.set_detail(detail, review);
                self.risk = restored.risk.map(Box::new);
                self.baseline = restored.baseline;
                self.insights = restored.insights.map(Box::new);
            }
            Outcome::Risk(_, report) => {
                if let Some(detail) = &self.detail
                    && let Some(mr) = self.queue.iter_mut().find(|mr| mr.id == detail.mr.id)
                {
                    mr.risk = Some(RiskSummary::from(report.as_ref()));
                }
                self.risk = Some(report);
                self.tab = Tab::Radar;
                self.scroll = 0;
                self.sort();
            }
            Outcome::RiskBaseline(_, baseline) => self.baseline = Some(baseline),
            Outcome::Review(_, review) => {
                self.review = Some(review);
                self.tab = Tab::Review;
                self.scroll = 0;
            }
            Outcome::Insights(_, insights) => {
                self.insights = Some(insights);
                self.tab = Tab::History;
                self.scroll = 0;
            }
            Outcome::Worktrees(worktrees, message) => {
                self.worktrees = worktrees;
                if let Some(message) = message {
                    self.status = message;
                }
            }
            Outcome::Approved(id, approvals) => {
                if let Some(detail) = &mut self.detail
                    && detail.mr.id == id
                {
                    detail.mr.approvals = approvals;
                }
                if let Some(mr) = self.queue.iter_mut().find(|mr| mr.id == id) {
                    mr.approvals = approvals;
                }
                self.status = "Approval recorded".into();
            }
            Outcome::Ask(ready) => {
                if self.demo {
                    self.status = "Demo: Claude is not launched".into();
                } else {
                    return vec![Effect::Chat(ready)];
                }
            }
            Outcome::SettingsSaved(config) => {
                let mut config = config;
                if self.demo {
                    config.languages = self.catalog.languages;
                }
                self.config = config;
                self.language_draft = self.config.languages;
                self.catalog.active = self.config.id.clone();
                self.queue.clear();
                self.detail = None;
                self.search.clear();
                self.selected = 0;
                self.screen = Screen::Inbox;
                return self.request(Action::Refresh);
            }
            Outcome::QueuePreferencesSaved(config, _) => {
                self.config.mr_roles = config.mr_roles;
                self.config.mr_sort = config.mr_sort;
                self.sort();
            }
            Outcome::Done(message) => self.status = message,
            _ => self.status = "Completed".into(),
        }
        vec![]
    }
    fn set_detail(&mut self, detail: Box<MrDetail>, review: Option<(ReviewLevel, Review)>) {
        self.detail = Some(detail);
        self.review = review.map(|(_, r)| r);
        self.risk = None;
        self.baseline = None;
        self.insights = None;
        self.tab = Tab::Description;
        self.scroll = 0;
        self.axis = 0;
    }
    pub fn languages_saved(
        &mut self,
        token: u64,
        languages: Languages,
        result: anyhow::Result<()>,
    ) {
        if !self
            .jobs
            .get(&Lane::Preferences)
            .is_some_and(|job| job.token == token)
        {
            return;
        }
        match result {
            Ok(()) => {
                if self.config.languages.information != languages.information {
                    self.review = None;
                }
                self.catalog.languages = languages;
                self.config.languages = languages;
                self.complete(token, Ok(Outcome::Done("Languages saved".into())));
            }
            Err(error) => {
                self.complete(token, Err(error));
            }
        }
    }
    fn save_languages(&mut self) -> Vec<Effect> {
        if self.busy() {
            return vec![];
        }
        let (token, mut effects) = self.begin(Lane::Preferences, "Saving…", None, true);
        effects.push(Effect::SaveLanguages {
            token,
            languages: self.language_draft,
        });
        effects
    }
    fn activate(&mut self, index: usize) -> Vec<Effect> {
        // An approval already sent must finish before switching projects.
        if self.jobs.values().any(|job| !job.cancellable) {
            return vec![];
        }
        let Some(project) = self.catalog.projects.get(index) else {
            return vec![];
        };
        let id = project.id.clone();
        let mut effects = self
            .jobs
            .drain()
            .map(|(_, job)| Effect::Cancel(job.token))
            .collect::<Vec<_>>();
        let (token, _) = self.begin(Lane::Navigation, "Loading project…", None, true);
        effects.push(Effect::Activate { token, id });
        effects
    }
    fn open_selected(&mut self) -> Vec<Effect> {
        let Some(id) = self.filtered().get(self.selected).map(|mr| mr.id.clone()) else {
            return vec![];
        };
        self.detail = None;
        self.risk = None;
        self.baseline = None;
        self.review = None;
        self.insights = None;
        self.screen = Screen::Reader;
        self.tab = Tab::Description;
        self.scroll = 0;
        self.request(Action::Select(id))
    }
    pub fn navigate(&mut self, navigation: Navigation) -> Vec<Effect> {
        self.scroll = 0;
        self.screen = match navigation {
            Navigation::Inbox => Screen::Inbox,
            Navigation::Projects => Screen::Projects,
            Navigation::Settings => Screen::Settings,
            Navigation::Worktrees => Screen::Worktrees,
        };
        if navigation == Navigation::Worktrees {
            return self.request(Action::Worktrees);
        }
        vec![]
    }
    fn scroll_by(&mut self, amount: i32) {
        self.scroll = (i32::from(self.scroll) + amount).clamp(0, i32::from(u16::MAX)) as u16;
    }
    pub fn mouse(&mut self, mouse: MouseEvent) -> Vec<Effect> {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                if self.screen == Screen::Inbox {
                    self.selected =
                        (self.selected + 1).min(self.filtered().len().saturating_sub(1));
                } else {
                    self.scroll_by(3);
                }
            }
            MouseEventKind::ScrollUp => {
                if self.screen == Screen::Inbox {
                    self.selected = self.selected.saturating_sub(1);
                } else {
                    self.scroll_by(-3);
                }
            }
            MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left)
                if self.confirmation.is_none() && !self.help =>
            {
                let hit = self
                    .hits
                    .iter()
                    .rev()
                    .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
                    .map(|(_, hit)| hit.clone());
                match hit {
                    Some(Hit::Nav(navigation)) => return self.navigate(navigation),
                    Some(Hit::Row(index)) => {
                        self.selected = index;
                        return self.open_selected();
                    }
                    Some(Hit::Tab(tab)) => {
                        self.tab = tab;
                        self.scroll = 0;
                    }
                    Some(Hit::Axis(axis)) => {
                        self.axis = axis;
                        self.scroll = 0;
                    }
                    Some(Hit::Project(index)) => return self.activate(index),
                    Some(Hit::Interface(language)) => self.language_draft.interface = language,
                    Some(Hit::Information(language)) => self.language_draft.information = language,
                    Some(Hit::SaveLanguages) => return self.save_languages(),
                    None => {}
                }
            }
            _ => {}
        }
        vec![]
    }
    pub fn key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if key.kind == ratatui::crossterm::event::KeyEventKind::Release {
            return vec![];
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return vec![Effect::Quit];
        }
        if self.help {
            self.help = false;
            return vec![];
        }
        if self.confirmation.is_some() {
            if key.code == KeyCode::Char('y') {
                return match self.confirmation.take().expect("confirmation") {
                    Confirmation::Approve(detail) => self.request(Action::Approve(detail)),
                    Confirmation::Tools(detail) => self.request(Action::SetupRiskTools(detail)),
                    Confirmation::Lint(detail, command) => {
                        self.request(Action::ApproveRiskCommand(detail, command))
                    }
                };
            }
            if matches!(key.code, KeyCode::Char('n') | KeyCode::Esc) {
                self.confirmation = None;
            }
            match key.code {
                KeyCode::Down | KeyCode::PageDown => self.scroll_by(3),
                KeyCode::Up | KeyCode::PageUp => self.scroll_by(-3),
                KeyCode::Home => self.scroll = 0,
                KeyCode::End => self.scroll = u16::MAX,
                _ => {}
            }
            return vec![];
        }
        if self.searching {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.searching = false,
                KeyCode::Backspace => {
                    self.search.pop();
                    self.selected = 0;
                }
                KeyCode::Char(c) => {
                    self.search.push(c);
                    self.selected = 0;
                }
                _ => {}
            }
            return vec![];
        }
        match key.code {
            KeyCode::Char('q') => {
                if self.jobs.values().any(|job| !job.cancellable) {
                    self.status = "Wait for the approval response before quitting.".into();
                } else {
                    return vec![Effect::Quit];
                }
            }
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('i') => return self.navigate(Navigation::Inbox),
            KeyCode::Char('p') => return self.navigate(Navigation::Projects),
            KeyCode::Char(',') => return self.navigate(Navigation::Settings),
            KeyCode::Char('w') => return self.navigate(Navigation::Worktrees),
            KeyCode::Esc => {
                if let Some(job) = self.jobs.get(&Lane::Operation)
                    && job.cancellable
                {
                    let token = job.token;
                    self.jobs.remove(&Lane::Operation);
                    self.status = "Cancelled".into();
                    return vec![Effect::Cancel(token)];
                }
                return self.navigate(Navigation::Inbox);
            }
            KeyCode::Char('r') if self.screen != Screen::Reader => {
                return self.request(Action::Refresh);
            }
            KeyCode::Down | KeyCode::Char('j') => match self.screen {
                Screen::Inbox => {
                    self.selected = (self.selected + 1).min(self.filtered().len().saturating_sub(1))
                }
                Screen::Projects => {
                    self.project_selected = (self.project_selected + 1)
                        .min(self.catalog.projects.len().saturating_sub(1))
                }
                _ => self.scroll_by(1),
            },
            KeyCode::Up | KeyCode::Char('k') => match self.screen {
                Screen::Inbox => self.selected = self.selected.saturating_sub(1),
                Screen::Projects => self.project_selected = self.project_selected.saturating_sub(1),
                _ => self.scroll_by(-1),
            },
            KeyCode::PageDown => self.scroll_by(12),
            KeyCode::PageUp => self.scroll_by(-12),
            KeyCode::Home => {
                self.scroll = 0;
                self.selected = 0;
            }
            KeyCode::End => {
                self.scroll = u16::MAX;
                self.selected = self.filtered().len().saturating_sub(1);
            }
            KeyCode::Enter if self.screen == Screen::Inbox => return self.open_selected(),
            KeyCode::Enter if self.screen == Screen::Projects => {
                return self.activate(self.project_selected);
            }
            KeyCode::Char('/') if self.screen == Screen::Inbox => {
                self.searching = true;
                self.search.clear();
                self.selected = 0;
            }
            KeyCode::Char('s') if self.screen == Screen::Inbox => {
                let index = QueueSort::ALL
                    .iter()
                    .position(|sort| *sort == self.config.mr_sort)
                    .unwrap_or(0);
                self.config.mr_sort = QueueSort::ALL[(index + 1) % QueueSort::ALL.len()];
                self.sort();
                self.selected = 0;
                if self.demo {
                    return vec![];
                }
                let (token, mut effects) = self.begin(Lane::Preferences, "Saving…", None, true);
                effects.push(Effect::SaveQueue {
                    token,
                    config: Box::new(self.config.clone()),
                });
                return effects;
            }
            KeyCode::Char('1'..='3') if self.screen == Screen::Inbox => {
                let KeyCode::Char(c) = key.code else {
                    unreachable!()
                };
                let role = mrde_core::config::MrRole::ALL[c as usize - '1' as usize];
                if self.config.mr_roles.contains(&role) {
                    if self.config.mr_roles.len() == 1 {
                        return vec![];
                    }
                    self.config.mr_roles.retain(|r| *r != role);
                } else {
                    self.config.mr_roles.push(role);
                }
                if self.demo {
                    return self.request(Action::Refresh);
                }
                let (token, mut effects) = self.begin(Lane::Preferences, "Saving…", None, true);
                effects.push(Effect::SaveQueue {
                    token,
                    config: Box::new(self.config.clone()),
                });
                effects.extend(self.request(Action::Refresh));
                return effects;
            }
            KeyCode::Char('1'..='5') if self.screen == Screen::Reader => {
                let KeyCode::Char(c) = key.code else {
                    unreachable!()
                };
                self.tab = Tab::ALL[c as usize - '1' as usize];
                self.scroll = 0;
            }
            KeyCode::Tab | KeyCode::BackTab if self.screen == Screen::Reader => {
                let index = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
                let step = if key.code == KeyCode::BackTab {
                    Tab::ALL.len() - 1
                } else {
                    1
                };
                self.tab = Tab::ALL[(index + step) % Tab::ALL.len()];
                self.scroll = 0;
            }
            KeyCode::Left | KeyCode::Right
                if self.screen == Screen::Reader && self.tab == Tab::Radar =>
            {
                self.axis = (self.axis
                    + if key.code == KeyCode::Left {
                        Axis::COUNT - 1
                    } else {
                        1
                    })
                    % Axis::COUNT;
                self.scroll = 0;
            }
            KeyCode::Char('1') if self.screen == Screen::Settings => {
                self.language_draft.interface = toggle(self.language_draft.interface)
            }
            KeyCode::Char('2') if self.screen == Screen::Settings => {
                self.language_draft.information = toggle(self.language_draft.information)
            }
            KeyCode::Char('s') if self.screen == Screen::Settings => return self.save_languages(),
            KeyCode::Char('l') if self.screen == Screen::Reader => {
                self.level = match self.level {
                    ReviewLevel::Quick => ReviewLevel::Standard,
                    ReviewLevel::Standard => ReviewLevel::Deep,
                    ReviewLevel::Deep => ReviewLevel::Quick,
                };
            }
            KeyCode::Char(c) if self.screen == Screen::Reader && !self.busy() => {
                let Some(detail) = self.detail.clone() else {
                    return vec![];
                };
                let action = match c {
                    'r' => Some(Action::Risk(detail)),
                    'm' => Some(Action::RiskBaseline(detail)),
                    'v' => Some(Action::Review(detail, self.level)),
                    'h' => Some(Action::Analyze(detail)),
                    'c' => {
                        if self.tab == Tab::Radar
                            && let Some(report) = &self.risk
                        {
                            Some(Action::AskRisk(
                                detail,
                                Axis::ALL[self.axis],
                                report.clone(),
                            ))
                        } else {
                            Some(Action::Ask(detail, self.review.clone()))
                        }
                    }
                    'a' => {
                        self.scroll = 0;
                        self.confirmation = Some(Confirmation::Approve(detail));
                        None
                    }
                    't' => {
                        self.scroll = 0;
                        self.confirmation = Some(Confirmation::Tools(detail));
                        None
                    }
                    'y' => {
                        if let Some(proposal) =
                            self.risk.as_ref().and_then(|r| r.pending_commands.first())
                        {
                            self.scroll = 0;
                            self.confirmation =
                                Some(Confirmation::Lint(detail, proposal.key.clone()));
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(action) = action {
                    return self.request(action);
                }
            }
            _ => {}
        }
        vec![]
    }
}
fn toggle(language: Language) -> Language {
    match language {
        Language::English => Language::French,
        Language::French => Language::English,
    }
}

//! Screenshots of the real application, for comparing it with `--ui-preview`.
//!
//! Only active with both `MRDE_SNAPSHOT_REAL=1` and `MRDE_SNAPSHOT`. It waits for the
//! startup refresh, then opens one MR (read-only GitLab requests) and selects a tab, or
//! opens the Worktrees (local listing) or Settings page, and signals the snapshot timer.
//! It never starts reviews, analyses, Ask Claude, approvals, or saves. `MRDE_SCENARIO` uses
//! the preview syntax (`queue`, `detail:<tab>`, `focus:<tab>`, `worktrees`, `settings`);
//! `MRDE_SCENARIO_INDEX` picks the queue position (default 0).
use super::{Cockpit, inbox::DetailTab};
use crate::backend::Action;
use gpui::{Context, point, px};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    WaitQueue,
    WaitResult,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Queue,
    Mr,
    Worktrees,
    Settings,
}

pub(super) struct Automation {
    stage: Stage,
    target: Target,
    index: usize,
    tab: DetailTab,
    focus: bool,
    scroll: Option<f32>,
}

impl Automation {
    pub(super) fn from_env() -> Option<Self> {
        if !requested() {
            return None;
        }
        let scenario = std::env::var("MRDE_SCENARIO").unwrap_or_default();
        let (name, argument) = scenario.split_once(':').unwrap_or((&scenario, ""));
        Some(Self {
            stage: Stage::WaitQueue,
            target: match name {
                "detail" | "focus" => Target::Mr,
                "worktrees" => Target::Worktrees,
                "settings" => Target::Settings,
                _ => Target::Queue,
            },
            index: std::env::var("MRDE_SCENARIO_INDEX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            tab: super::preview::tab(argument),
            focus: name == "focus",
            scroll: std::env::var("MRDE_SCENARIO_SCROLL")
                .ok()
                .and_then(|v| v.parse().ok()),
        })
    }
}

fn ready() {
    #[cfg(target_os = "macos")]
    crate::snapshot::READY.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn requested() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::snapshot::real_requested()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

impl Cockpit {
    pub(super) fn drive_automation(&mut self, cx: &mut Context<Self>) {
        let Some(automation) = self.automation.as_mut() else {
            return;
        };
        let idle = self.job.is_none() && self.navigation_job.is_none() && !self.initializing;
        match (automation.stage, automation.target) {
            // The startup refresh has run once (successfully or not) and nothing is busy.
            (Stage::WaitQueue, target)
                if idle && self.last_refresh.elapsed() < Duration::from_secs(3600) =>
            {
                automation.stage = Stage::WaitResult;
                match target {
                    Target::Queue => {
                        automation.stage = Stage::Done;
                        ready();
                    }
                    Target::Settings => {
                        automation.stage = Stage::Done;
                        self.settings = true;
                        self.worktrees_view = false;
                        ready();
                        cx.notify();
                    }
                    Target::Worktrees => {
                        self.settings = false;
                        self.worktrees_view = true;
                        self.start(Action::Worktrees, "Listing worktrees…", cx);
                    }
                    Target::Mr => match self.queue.get(automation.index).map(|mr| mr.id.clone()) {
                        Some(id) => self.open_mr(id, cx),
                        None => {
                            automation.stage = Stage::Done;
                            ready();
                        }
                    },
                }
            }
            (Stage::WaitResult, Target::Worktrees) if idle => {
                automation.stage = Stage::Done;
                ready();
            }
            (Stage::WaitResult, Target::Mr) if idle && self.detail.is_some() => {
                let (tab, focus, scroll) = (automation.tab, automation.focus, automation.scroll);
                automation.stage = Stage::Done;
                self.show_detail_tab(tab);
                self.reader_focused = focus;
                if let Some(offset) = scroll {
                    self.detail_scroll.set_offset(point(px(0.), px(-offset)));
                }
                ready();
                cx.notify();
            }
            _ => {}
        }
    }
}

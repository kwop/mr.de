//! `--ui-preview` scenarios: put the interface in a given state with demo data, for visual
//! checks and screenshots. `MRDE_SCENARIO` examples: `queue`, `detail:files`,
//! `detail:review`, `focus:statistics`, `confirm`, `worktrees`, `settings`, `logs`.
use super::{Cockpit, inbox::DetailTab};
use crate::demo;
use gpui::Context;
use mrde_core::models::ReviewLevel;
use std::time::Instant;

pub(super) fn tab(key: &str) -> DetailTab {
    match key {
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
    pub(super) fn apply_preview_scenario(&mut self, _cx: &mut Context<Self>) {
        let scenario = std::env::var("MRDE_SCENARIO").unwrap_or_default();
        self.apply_preview_scenario_named(&scenario);
    }
    pub(super) fn apply_preview_scenario_named(&mut self, scenario: &str) {
        let scenario = scenario.to_string();
        self.config.gitlab_url = demo::INSTANCE.into();
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
                this.insights = Some((id.clone(), Box::new(demo::insights(&detail))));
                this.tour = Some((id.clone(), detail.mr.sha.clone(), demo::tour()));
                if let Ok(report) = demo::visual() {
                    this.visual = Some((id, detail.mr.sha.clone(), report));
                }
            }
            this.risk_colors = std::env::var("MRDE_SCENARIO_RISK").is_ok();
            this.review_level = Some(ReviewLevel::Deep);
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
            "settings" => self.settings = true,
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
        self.status = format!("UI preview — demo data, no network · scenario: {scenario}");
    }
}

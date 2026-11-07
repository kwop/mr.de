//! Project-scoped risk orchestration, always called on the backend runtime.
use super::*;
use mrde_core::risk::*;
use mrde_risk::config::RiskConfig;
use std::{collections::HashSet, path::Path};
fn approvals_key(config: &Config) -> String {
    format!("risk-approvals:{}", config.namespace())
}
fn config_for(env: &Env, config: &Config, path: &Path) -> anyhow::Result<RiskConfig> {
    let approvals: HashSet<String> = env
        .store()?
        .cached(&approvals_key(config))?
        .unwrap_or_default();
    RiskConfig::load(&env.settings, &config.id, path, &approvals)
}
async fn prepare(env: &Env, config: &Config, detail: &MrDetail) -> anyhow::Result<PathBuf> {
    let trees = Worktrees::new(&env.cache, &config.namespace());
    match trees.existing(detail) {
        Some(path) => Ok(path),
        None => trees.prepare(detail).await,
    }
}
fn baseline_key(config: &Config, project: u64, risk: &RiskConfig) -> String {
    format!(
        "risk-baseline:{}:{project}:{}",
        config.namespace(),
        risk.fingerprint()
    )
}
pub(super) fn cached(
    env: &Env,
    config: &Config,
    detail: &MrDetail,
) -> anyhow::Result<Option<RiskReport>> {
    let Some(path) = Worktrees::new(&env.cache, &config.namespace()).existing(detail) else {
        return Ok(None);
    };
    let risk = config_for(env, config, &path)?;
    Ok(env
        .store()?
        .risk(
            &config.namespace(),
            &detail.mr.id,
            &detail.mr.sha,
            &detail.mr.base_sha,
            &risk.fingerprint(),
        )?
        .filter(|r| r.profile.base_sha == detail.mr.base_sha))
}
pub(super) fn baseline_cached(
    env: &Env,
    config: &Config,
    detail: &MrDetail,
) -> anyhow::Result<Option<Baseline>> {
    let Some(path) = Worktrees::new(&env.cache, &config.namespace()).existing(detail) else {
        return Ok(None);
    };
    let risk = config_for(env, config, &path)?;
    env.store()?
        .cached(&baseline_key(config, detail.mr.id.project, &risk))
}
pub(super) fn hydrate_queue(
    env: &Env,
    config: &Config,
    queue: &mut [MergeRequest],
) -> anyhow::Result<()> {
    for mr in queue {
        let detail = MrDetail {
            mr: mr.clone(),
            files: vec![],
            issues: vec![],
            ssh_url: String::new(),
        };
        mr.risk = cached(env, config, &detail)
            .ok()
            .flatten()
            .as_ref()
            .map(RiskSummary::from);
    }
    Ok(())
}
pub(super) async fn calculate(
    env: &Env,
    config: &Config,
    detail: &MrDetail,
) -> anyhow::Result<Outcome> {
    let path = prepare(env, config, detail).await?;
    let risk = config_for(env, config, &path)?;
    let context = mrde_risk::context(&path, &detail.mr.base_sha, &detail.mr.sha, risk).await?;
    finish(env, config, detail, context).await
}
async fn finish(
    env: &Env,
    config: &Config,
    detail: &MrDetail,
    context: mrde_risk::MrContext,
) -> anyhow::Result<Outcome> {
    let report = mrde_risk::analyze(context).await;
    let store = env.store()?;
    store.save_risk(&config.namespace(), &detail.mr.id, &report)?;
    let mut queue = store.queue(&config.namespace())?;
    for mr in &mut queue {
        if mr.id == detail.mr.id && mr.sha == detail.mr.sha {
            mr.risk = Some(RiskSummary::from(&report));
        }
    }
    store.save_queue(&config.namespace(), &queue)?;
    Ok(Outcome::Risk(detail.mr.id.clone(), Box::new(report)))
}
pub(super) async fn setup(
    env: &Env,
    config: &Config,
    detail: &MrDetail,
) -> anyhow::Result<Outcome> {
    let path = prepare(env, config, detail).await?;
    let risk = config_for(env, config, &path)?;
    let mut context = mrde_risk::context(&path, &detail.mr.base_sha, &detail.mr.sha, risk).await?;
    mrde_risk::setup::prepare(&mut context, &env.cache).await;
    finish(env, config, detail, context).await
}
pub(super) async fn approve_command(
    env: &Env,
    config: &Config,
    detail: &MrDetail,
    key: &str,
) -> anyhow::Result<Outcome> {
    let path = prepare(env, config, detail).await?;
    let risk = config_for(env, config, &path)?;
    let mut context = mrde_risk::context(&path, &detail.mr.base_sha, &detail.mr.sha, risk).await?;
    let command = context
        .config
        .commands_mut()
        .find(|c| c.key() == key)
        .context("The command changed. Recalculate risks before approving it.")?;
    command.approved = true;
    let store = env.store()?;
    let mut approvals: HashSet<String> = store.cached(&approvals_key(config))?.unwrap_or_default();
    approvals.insert(key.into());
    store.save_cached(&approvals_key(config), &approvals)?;
    drop(store);
    mrde_risk::setup::prepare(&mut context, &env.cache).await;
    finish(env, config, detail, context).await
}
pub(super) async fn file(
    env: &Env,
    config: &Config,
    detail: &MrDetail,
    path: &str,
) -> anyhow::Result<Outcome> {
    let root = prepare(env, config, detail).await?;
    let context =
        mrde_git::risk::file_context(&root, &detail.mr.base_sha, &detail.mr.sha, path).await?;
    Ok(Outcome::RiskFile(
        detail.mr.id.clone(),
        detail.mr.sha.clone(),
        context,
    ))
}
pub(super) async fn baseline(
    env: &Env,
    config: &Config,
    detail: &MrDetail,
) -> anyhow::Result<Outcome> {
    let path = prepare(env, config, detail).await?;
    let risk = config_for(env, config, &path)?;
    let ids = env
        .forge
        .merged_history(detail.mr.id.project, risk.baseline_count)
        .await?;
    let mut profiles = Vec::new();
    let mut warnings = Vec::new();
    // Use the current repository's local object database. No hidden network Git fetches:
    // missing historical commits are reported explicitly and never counted as zero.
    for id in &ids {
        let result = async {
            let historical = env.forge.summary(id).await?;
            let mut historical_config = risk.clone();
            historical_config.coverage_path.clear();
            if let Some(cached) = env.store()?.risk(
                &config.namespace(),
                id,
                &historical.sha,
                &historical.base_sha,
                &historical_config.fingerprint(),
            )? {
                return Ok::<_, anyhow::Error>(cached.profile);
            }
            let context = mrde_risk::context(
                &path,
                &historical.base_sha,
                &historical.sha,
                historical_config,
            )
            .await?;
            let report = mrde_risk::analyze(context).await;
            env.store()?.save_risk(&config.namespace(), id, &report)?;
            Ok(report.profile)
        }
        .await;
        match result {
            Ok(profile) => profiles.push(profile),
            Err(error) => warnings.push(format!("!{}: {error:#}", id.iid)),
        }
    }
    let mut baseline = Baseline::from_profiles(&profiles, risk.baseline_count, ids.len());
    baseline.warnings = warnings;
    if !risk.coverage_path.is_empty() {
        baseline.warnings.push("Historical test gaps use the file-ratio heuristic; current coverage reports do not describe historical commits.".into());
    }
    env.store()?.save_cached(
        &baseline_key(config, detail.mr.id.project, &risk),
        &baseline,
    )?;
    Ok(Outcome::RiskBaseline(detail.mr.id.clone(), baseline))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(directory: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(directory)
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
    fn commit(directory: &Path, files: &[(&str, &str)], message: &str) -> String {
        for (path, text) in files {
            let path = directory.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        git(directory, &["add", "."]);
        git(directory, &["commit", "-qm", message]);
        git(directory, &["rev-parse", "HEAD"])
    }
    fn mr(iid: u64, base: &str, sha: &str) -> MergeRequest {
        serde_json::from_value(serde_json::json!({
            "id": {"project": 1, "iid": iid}, "title": format!("MR {iid}"), "description": "",
            "project": "team/project", "author": "alice",
            "web_url": format!("https://gitlab.example.com/team/project/-/merge_requests/{iid}"),
            "sha": sha, "base_sha": base, "created_at": "2026-11-01T00:00:00Z", "labels": [],
            "pipeline": null, "approvals": null, "changed_lines": null
        }))
        .unwrap()
    }
    /// Answers merged history and summaries from a list, most recently merged first.
    struct History(Vec<MergeRequest>);
    #[async_trait]
    impl Forge for History {
        async fn merged_history(&self, _: u64, limit: usize) -> anyhow::Result<Vec<MrId>> {
            Ok(self.0.iter().take(limit).map(|mr| mr.id.clone()).collect())
        }
        async fn summary(&self, id: &MrId) -> anyhow::Result<MergeRequest> {
            self.0
                .iter()
                .find(|mr| mr.id == *id)
                .cloned()
                .context("Unknown MR")
        }
        async fn test_connection(&self) -> anyhow::Result<String> {
            Ok("me".into())
        }
        async fn queue(
            &self,
            _: &[mrde_core::config::MrRole],
        ) -> anyhow::Result<Vec<MergeRequest>> {
            Ok(vec![])
        }
        async fn approval_count(&self, _: &MrId) -> anyhow::Result<Option<u64>> {
            Ok(None)
        }
        async fn detail(&self, _: &MrId) -> anyhow::Result<MrDetail> {
            anyhow::bail!("Not used by risk actions")
        }
        async fn approve(&self, _: &MrId, _: &str) -> anyhow::Result<()> {
            anyhow::bail!("Not used by risk actions")
        }
    }

    #[tokio::test]
    async fn changes_measure_the_mr_its_merged_history_and_file_history_locally() {
        let temp = tempfile::tempdir().unwrap();
        let origin = temp.path().join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q"]);
        git(&origin, &["config", "user.email", "developer@example.com"]);
        git(&origin, &["config", "user.name", "Developer"]);
        let initial = commit(&origin, &[("src/lib.rs", "pub fn lib() {}\n")], "Initial");
        let tested = commit(
            &origin,
            &[
                ("src/user.rs", "pub fn user() {}\n"),
                ("tests/user.rs", "#[test]\nfn user() {}\n"),
            ],
            "Add users",
        );
        let untested = commit(
            &origin,
            &[("src/report.rs", &"pub fn report() {}\n".repeat(40))],
            "Add reports",
        );
        let head = commit(
            &origin,
            &[(
                "src/user.rs",
                "pub fn user() {}\npub fn rename() { load().unwrap(); }\n",
            )],
            "Rename users",
        );
        git(
            &origin,
            &["update-ref", "refs/merge-requests/20/head", &head],
        );
        let config = Config {
            gitlab_url: "https://gitlab.example.com".into(),
            ..Default::default()
        };
        // The project's mirror points at the local origin instead of an SSH remote.
        let cache = temp.path().join("cache");
        let repository = cache
            .join("git")
            .join(mrde_git::instance_key(&config.namespace()))
            .join("1.git");
        std::fs::create_dir_all(&repository).unwrap();
        git(&repository, &["init", "-q", "--bare"]);
        git(
            &repository,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        let detail = MrDetail {
            mr: mr(20, &untested, &head),
            files: vec![],
            issues: vec![],
            ssh_url: "git@example.invalid:team/project.git".into(),
        };
        let env = Env {
            forge: Arc::new(History(vec![
                mr(11, &tested, &untested),
                mr(10, &initial, &tested),
                mr(12, &initial, &"f".repeat(40)),
            ])),
            cache,
            settings: temp.path().join("config"),
        };

        let Outcome::Risk(_, report) =
            run(&env, config.clone(), Action::Risk(Box::new(detail.clone())))
                .await
                .unwrap()
        else {
            panic!("expected a risk report");
        };
        let value = |axis: Axis| {
            report.profile.axes[axis.index()]
                .raw
                .as_ref()
                .map(|r| r.value)
        };
        assert_eq!(value(Axis::DiffSize), Some(1.));
        assert_eq!(
            value(Axis::TestGap),
            Some(1.),
            "one added line, no test change"
        );
        assert_eq!(value(Axis::Debt), Some(1.));
        assert_eq!(value(Axis::Coupling), Some(0.));
        assert_eq!(report.profile.axes[Axis::Lint.index()].score, None);
        assert_eq!(
            report.profile.verdict(&report.verdict_thresholds),
            Some(Verdict::Quick)
        );
        assert!(cached(&env, &config, &detail).unwrap().is_some());

        let Outcome::RiskBaseline(_, baseline) = run(
            &env,
            config.clone(),
            Action::RiskBaseline(Box::new(detail.clone())),
        )
        .await
        .unwrap() else {
            panic!("expected a baseline");
        };
        assert_eq!(baseline.merged_mrs, 3);
        assert_eq!(
            baseline.samples[Axis::DiffSize.index()],
            2,
            "a commit missing locally is not counted as zero"
        );
        assert_eq!(baseline.warnings.len(), 1);
        assert!(
            baseline.warnings[0].starts_with("!12"),
            "{:?}",
            baseline.warnings
        );
        // The tested MR has no gap; the other adds 40 untested lines (2.6): median 1.3.
        let gap = baseline.medians[Axis::TestGap.index()].unwrap();
        assert!((gap - 1.3).abs() < 0.01, "{gap}");

        let Outcome::RiskFile(_, _, file) = run(
            &env,
            config,
            Action::RiskFile(Box::new(detail), "src/user.rs".into()),
        )
        .await
        .unwrap() else {
            panic!("expected the file history");
        };
        let own: Vec<bool> = file.commits.iter().map(|c| c.in_mr).collect();
        assert_eq!(own, [true, false], "the MR's commit, then the earlier one");
        assert_eq!(file.authors, [("Developer".to_string(), 100.)]);
    }
}

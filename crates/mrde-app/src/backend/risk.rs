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
    RiskConfig::load(&config_dir()?, &config.id, path, &approvals)
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
pub(super) async fn approve_command(
    env: &Env,
    config: &Config,
    detail: &MrDetail,
    key: &str,
) -> anyhow::Result<Outcome> {
    let path = prepare(env, config, detail).await?;
    let risk = config_for(env, config, &path)?;
    anyhow::ensure!(
        std::iter::once(&risk.complexity)
            .chain(&risk.lint)
            .any(|c| c.key() == key),
        "The command changed. Recalculate risks before approving it."
    );
    let store = env.store()?;
    let mut approvals: HashSet<String> = store.cached(&approvals_key(config))?.unwrap_or_default();
    approvals.insert(key.into());
    store.save_cached(&approvals_key(config), &approvals)?;
    drop(store);
    calculate(env, config, detail).await
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

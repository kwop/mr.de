use super::*;
use mrde_core::{models::MrId, risk::RiskReport};
impl Store {
    pub fn save_risk(&self, namespace: &str, id: &MrId, report: &RiskReport) -> anyhow::Result<()> {
        let p = &report.profile;
        self.connection.execute("INSERT INTO risk_profiles(namespace,project_id,iid,base_sha,head_sha,config_key,computed_at,payload) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(namespace,project_id,iid,base_sha,head_sha,config_key) DO UPDATE SET computed_at=excluded.computed_at,payload=excluded.payload",params![namespace,id.project,id.iid,p.base_sha,p.head_sha,p.config_key,p.computed_at.to_rfc3339(),serde_json::to_string(report)?])?;
        Ok(())
    }
    pub fn risk(
        &self,
        namespace: &str,
        id: &MrId,
        head: &str,
        base: &str,
        config: &str,
    ) -> anyhow::Result<Option<RiskReport>> {
        let text:Option<String>=self.connection.query_row("SELECT payload FROM risk_profiles WHERE namespace=?1 AND project_id=?2 AND iid=?3 AND head_sha=?4 AND config_key=?5 AND base_sha=?6 ORDER BY computed_at DESC LIMIT 1",params![namespace,id.project,id.iid,head,config,base],|r|r.get(0)).optional()?;
        text.map(|s| serde_json::from_str(&s))
            .transpose()
            .context("Invalid cached risk profile")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use mrde_core::risk::*;
    #[test]
    fn profiles_are_isolated_by_project_commit_and_configuration() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let id = MrId { project: 1, iid: 2 };
        let report = RiskReport {
            profile: RiskProfile {
                axes: std::array::from_fn(|i| AxisScore::unavailable(Axis::ALL[i], "fixture")),
                computed_at: "2026-01-01T00:00:00Z".parse().unwrap(),
                head_sha: "head".into(),
                base_sha: "base".into(),
                config_key: "config-v1".into(),
            },
            verdict_thresholds: VerdictThresholds::default(),
            stats: mrde_core::models::DiffStats {
                sha: "head".into(),
                source: mrde_core::models::StatsSource::Git,
                files: vec![],
            },
            excluded: vec![],
            pending_commands: vec![],
        };
        store.save_risk("project-a", &id, &report).unwrap();
        // Opening the same database concurrently can observe the old schema version.
        store
            .connection
            .execute_batch(include_str!("../migrations/003_risk.sql"))
            .unwrap();
        assert!(
            store
                .risk("project-a", &id, "head", "base", "config-v1")
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .risk("project-a", &id, "head", "another-base", "config-v1")
                .unwrap()
                .is_none()
        );
        for (namespace, head, key) in [
            ("project-b", "head", "config-v1"),
            ("project-a", "new-head", "config-v1"),
            ("project-a", "head", "config-v2"),
        ] {
            assert!(
                store
                    .risk(namespace, &id, head, "base", key)
                    .unwrap()
                    .is_none()
            );
        }
    }
}

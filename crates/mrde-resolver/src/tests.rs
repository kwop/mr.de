use super::*;
use std::sync::Mutex;

struct Fake {
    remote: Mutex<String>,
    repairs: Mutex<usize>,
    published: Mutex<Vec<ResolverThread>>,
    statuses: Mutex<std::collections::VecDeque<&'static str>>,
    conflict: bool,
    question: bool,
    stale_pipeline: bool,
}
impl Fake {
    fn new(statuses: &[&'static str]) -> Self {
        Self {
            remote: Mutex::new("a".repeat(40)),
            repairs: Mutex::new(0),
            published: Mutex::new(vec![]),
            statuses: Mutex::new(statuses.iter().copied().collect()),
            conflict: false,
            question: false,
            stale_pipeline: false,
        }
    }
}
fn thread() -> ResolverThread {
    new_thread(
        "https://gitlab.example.com".into(),
        mrde_core::models::MrId { project: 1, iid: 2 },
        "Fix the queue".into(),
        "a".repeat(40),
        "{}".into(),
    )
}
#[async_trait]
impl Services for Fake {
    async fn target(&self, thread: &ResolverThread) -> anyhow::Result<ResolverTarget> {
        Ok(ResolverTarget {
            id: thread.mr.clone(),
            source_project: 1,
            source_branch: "fix".into(),
            target_branch: "main".into(),
            source_ssh: "git@example.com:app.git".into(),
            target_ssh: "git@example.com:app.git".into(),
            head_sha: self.remote.lock().unwrap().clone(),
            target_sha: "b".repeat(40),
        })
    }
    async fn prepare(&self, _: &ResolverThread, _: &ResolverTarget) -> anyhow::Result<PathBuf> {
        Ok("/synthetic/worktree".into())
    }
    async fn head(&self, _: &ResolverThread) -> anyhow::Result<String> {
        Ok(self.remote.lock().unwrap().clone())
    }
    async fn rebase(&self, _: &ResolverThread, _: &ResolverTarget) -> anyhow::Result<bool> {
        Ok(!self.conflict || *self.repairs.lock().unwrap() > 0)
    }
    async fn publish_head(&self, _: &ResolverThread, _: &ResolverTarget) -> anyhow::Result<String> {
        let n = *self.repairs.lock().unwrap();
        let sha = if n == 0 {
            "a".repeat(40)
        } else {
            format!("{n:040x}")
        };
        *self.remote.lock().unwrap() = sha.clone();
        Ok(sha)
    }
    async fn pipeline(&self, _: &ResolverTarget, sha: &str) -> anyhow::Result<Option<Pipeline>> {
        let status = self
            .statuses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or("failed");
        if status == "absent" {
            return Ok(None);
        }
        Ok(Some(Pipeline {
            id: 4,
            project: 1,
            sha: if self.stale_pipeline {
                "old".into()
            } else {
                sha.into()
            },
            status: status.into(),
            web_url: String::new(),
        }))
    }
    async fn logs(&self, _: &Pipeline) -> anyhow::Result<Vec<JobLog>> {
        Ok(vec![JobLog {
            name: "test".into(),
            trace: "assertion failed".into(),
        }])
    }
    async fn repair(
        &self,
        _: &ResolverThread,
        _: &str,
        prompt: String,
    ) -> anyhow::Result<mrde_claude::resolver::RepairRun> {
        *self.repairs.lock().unwrap() += 1;
        assert!(prompt.contains("untrusted data"));
        Ok(mrde_claude::resolver::RepairRun {
            result: RepairResult {
                ready: !self.question,
                summary: "Fixed the cause".into(),
                question: self
                    .question
                    .then(|| "Should an empty queue be valid?".into()),
            },
            output: "Investigating the failing assertion\n".into(),
        })
    }
    async fn publish(&self, thread: &ResolverThread) -> anyhow::Result<()> {
        self.published.lock().unwrap().push(thread.clone());
        Ok(())
    }
    async fn wait(&self) {}
}

#[tokio::test]
async fn rebases_repairs_then_tracks_the_new_commit_until_green() {
    let fake = Fake {
        conflict: true,
        ..Fake::new(&["pending", "failed", "running", "success"])
    };
    let result = run(&fake, thread()).await.unwrap();
    assert_eq!(result.state, ResolverState::Succeeded);
    assert_eq!(result.attempts, 2);
    assert_eq!(result.sessions.len(), 2);
    assert!(
        result
            .sessions
            .iter()
            .all(|s| s.finished_at.is_some() && !s.output.is_empty())
    );
    assert_eq!(result.pipeline.unwrap().sha, result.remote_sha);
    assert!(
        fake.published
            .lock()
            .unwrap()
            .windows(2)
            .all(|p| p[0].sequence <= p[1].sequence)
    );
}
#[tokio::test]
async fn never_starts_a_fourth_repair_without_manual_resume() {
    let fake = Fake::new(&[]);
    let mut result = run(&fake, thread()).await.unwrap();
    assert_eq!(result.state, ResolverState::NeedsInput);
    assert_eq!(*fake.repairs.lock().unwrap(), 3);
    assert_eq!(result.sessions.len(), 3);
    assert!(
        result
            .question
            .as_ref()
            .unwrap()
            .contains("Three repair attempts")
    );
    result.resume();
    fake.statuses.lock().unwrap().extend(["failed", "success"]);
    let result = run(&fake, result).await.unwrap();
    assert_eq!(result.state, ResolverState::Succeeded);
    assert_eq!(result.attempts, 1);
    assert_eq!(result.sessions.len(), 4);
    assert_eq!(result.generation, 2);
}
#[tokio::test]
async fn question_immediately_stops_without_publishing_a_correction() {
    let fake = Fake {
        question: true,
        ..Fake::new(&["failed"])
    };
    let result = run(&fake, thread()).await.unwrap();
    assert_eq!(result.state, ResolverState::NeedsInput);
    assert_eq!(result.attempts, 1);
    assert_eq!(result.remote_sha, "a".repeat(40));
    assert_eq!(
        result.question.as_deref(),
        Some("Should an empty queue be valid?")
    );
}
#[tokio::test]
async fn green_pipeline_of_another_sha_is_never_accepted() {
    let fake = Fake {
        stale_pipeline: true,
        ..Fake::new(&["success"])
    };
    let result = run(&fake, thread()).await.unwrap();
    assert_eq!(result.state, ResolverState::NeedsInput);
    assert_eq!(result.attempts, 0);
}
#[tokio::test]
async fn stale_selection_stops_before_worktree_or_claude() {
    let fake = Fake::new(&[]);
    *fake.remote.lock().unwrap() = "c".repeat(40);
    let result = run(&fake, thread()).await.unwrap();
    assert_eq!(result.state, ResolverState::NeedsInput);
    assert!(result.worktree.is_none());
    assert!(result.sessions.is_empty());
}
#[tokio::test]
async fn manual_and_missing_pipelines_require_input_without_repairing() {
    for statuses in [vec!["manual"], vec!["absent"; 21], vec!["running"; 241]] {
        let fake = Fake::new(&statuses);
        let result = run(&fake, thread()).await.unwrap();
        assert_eq!(result.state, ResolverState::NeedsInput);
        assert!(result.sessions.is_empty());
    }
}
#[test]
fn persisted_threads_reject_old_generations_and_out_of_order_snapshots() {
    let store = mrde_store::Store::open(Path::new(":memory:")).unwrap();
    let mut old = thread();
    old.event("Running");
    store.save_thread(&old).unwrap();
    let mut stopped = old.clone();
    stopped.generation += 1;
    stopped.state = ResolverState::Stopped;
    stopped.event("Stopped");
    store.save_thread(&stopped).unwrap();
    old.event("Late result");
    store.save_thread(&old).unwrap();
    assert_eq!(
        store.threads(&old.instance).unwrap()[0].state,
        ResolverState::Stopped
    );
    assert!(
        store
            .threads("https://another.example.com")
            .unwrap()
            .is_empty()
    );
    let mut new = stopped.clone();
    new.event("Latest event");
    store.save_thread(&new).unwrap();
    store.save_thread(&stopped).unwrap();
    assert_eq!(
        store.threads(&old.instance).unwrap()[0].sequence,
        new.sequence
    );
}
#[test]
fn threads_saved_before_kinds_existed_are_resolver_runs() {
    let mut value = serde_json::to_value(thread()).unwrap();
    value.as_object_mut().unwrap().remove("kind");
    let old: ResolverThread = serde_json::from_value(value.clone()).unwrap();
    assert!(old.automated());
    assert_eq!(old.label(), "Preparing");
    value["kind"] = "questions".into();
    let questions: ResolverThread = serde_json::from_value(value).unwrap();
    assert!(!questions.automated());
    assert_eq!(questions.label(), "Questions");
}

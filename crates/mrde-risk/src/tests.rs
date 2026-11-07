use super::*;
use crate::{
    config::{Command, SensitiveGlob},
    scorers::*,
};
use std::process::Command as Process;

const DIFF: &str = "diff --git a/src/auth.rs b/src/auth.rs\n--- a/src/auth.rs\n+++ b/src/auth.rs\n@@ -1 +1,3 @@\n-fn old() {}\n+use crate::session;\n+fn login() { session::run().unwrap(); }\n+// TODO check expiry\ndiff --git a/tests/auth.rs b/tests/auth.rs\nnew file mode 100644\n--- /dev/null\n+++ b/tests/auth.rs\n@@ -0,0 +1 @@\n+#[test] fn login_test() {}\n";
fn fixture() -> MrContext {
    let files = mrde_insights::diff::parse(DIFF);
    let stats = DiffStats {
        sha: "b".repeat(40),
        source: StatsSource::Git,
        files: files
            .iter()
            .map(|f| mrde_core::models::FileStat {
                path: f.path().into(),
                added: f.added() as u64,
                removed: f.removed() as u64,
                binary: f.binary,
            })
            .collect(),
    };
    let contents = files
        .iter()
        .map(|f| {
            (
                f.path().to_string(),
                (
                    Some(
                        f.hunks
                            .iter()
                            .flat_map(|h| h.removed.clone())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                    Some(
                        f.hunks
                            .iter()
                            .flat_map(|h| h.added.clone())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                ),
            )
        })
        .collect();
    let config = RiskConfig {
        sensitive: vec![
            SensitiveGlob {
                glob: "src/auth*".into(),
                weight: 1.,
            },
            SensitiveGlob {
                glob: "src/**".into(),
                weight: 0.5,
            },
        ],
        ..Default::default()
    };
    MrContext {
        worktree: "/workspace".into(),
        base_sha: "a".repeat(40),
        head_sha: "b".repeat(40),
        files,
        stats,
        excluded: vec![],
        contents,
        contents_error: None,
        coverage: None,
        coverage_root: "/workspace".into(),
        config,
        snapshots: None,
        snapshot_error: None,
        environment: mrde_checks::environment::Environment::default(),
        setup_messages: vec![],
        check_errors: Default::default(),
        detection_error: None,
        prepared_checks: Default::default(),
    }
}
#[tokio::test]
async fn diff_counts_added_and_removed_lines_and_files_count_renames_once() {
    let ctx = fixture();
    assert_eq!(DiffScorer.measure(&ctx).await.unwrap().value, 5.);
    assert_eq!(FileScorer.measure(&ctx).await.unwrap().value, 2.);
}
#[tokio::test]
async fn sensitive_paths_use_the_highest_weight_without_double_counting() {
    let raw = SensitivePathScorer.measure(&fixture()).await.unwrap();
    assert_eq!(raw.value, 0.8);
    assert_eq!(raw.evidence.len(), 1);
}
#[tokio::test]
async fn test_gap_distinguishes_heuristics_from_lcov_and_cobertura() {
    let mut ctx = fixture();
    let heuristic = TestGapScorer.measure(&ctx).await.unwrap();
    assert_eq!(heuristic.value, 0.);
    assert!(heuristic.estimated);
    ctx.coverage = Some("SF:/workspace/src/auth.rs\nDA:2,0\nDA:3,1\nend_of_record\n".into());
    assert_eq!(TestGapScorer.measure(&ctx).await.unwrap().value, 1.);
    ctx.coverage=Some(r#"<coverage><packages><package><classes><class filename="src/auth.rs"><lines><line number="2" hits="0"/><line number="3" hits="1"/></lines></class></classes></package></packages></coverage>"#.into());
    assert_eq!(TestGapScorer.measure(&ctx).await.unwrap().value, 1.);
    ctx.coverage = Some("malformed report".into());
    assert!(TestGapScorer.measure(&ctx).await.is_err());
}
#[tokio::test]
async fn debt_counts_only_added_lines_and_preserves_line_evidence() {
    let raw = DebtScorer.measure(&fixture()).await.unwrap();
    assert_eq!(raw.value, 2.);
    assert_eq!(raw.evidence[0].line, Some(2));
}
#[tokio::test]
async fn coupling_counts_new_targets_and_manifest_dependencies_in_four_ecosystems() {
    let mut ctx = fixture();
    assert_eq!(CouplingScorer.measure(&ctx).await.unwrap().value, 1.);
    for (path, before, after) in [
        ("Cargo.toml", "[dependencies]", "[dependencies]\nserde='1'"),
        ("package.json", "{}", r#"{"dependencies":{"library":"1"}}"#),
        (
            "pubspec.yaml",
            "name: app",
            "name: app\ndependencies:\n  library: any",
        ),
        (
            "go.mod",
            "module example.com/app",
            "module example.com/app\nrequire example.com/lib v1.0.0",
        ),
    ] {
        ctx.files = vec![FileDiff {
            old_path: Some(path.into()),
            new_path: Some(path.into()),
            ..Default::default()
        }];
        ctx.contents = BTreeMap::from([(path.into(), (Some(before.into()), Some(after.into())))]);
        assert_eq!(
            CouplingScorer.measure(&ctx).await.unwrap().value,
            1.,
            "{path}"
        );
    }
}
#[test]
fn related_tests_match_names_module_directories_and_generic_test_modules() {
    for (test, source, expected) in [
        ("tests/auth.rs", "src/auth.rs", true),
        (
            "api/create_schedule_test.go",
            "api/create_schedule.go",
            true,
        ),
        (
            "web/__tests__/ScheduleDialog.test.tsx",
            "web/ScheduleDialog.tsx",
            true,
        ),
        (
            "app/src/test/InvoiceServiceTest.kt",
            "app/src/main/InvoiceService.kt",
            true,
        ),
        ("src/github/tests.rs", "src/github.rs", true),
        ("src/settings/projects_tests.rs", "src/settings.rs", true),
        ("src/ui/tests.rs", "src/ui/mod.rs", true),
        ("src/tests.rs", "src/scorers.rs", true),
        ("tests/auth.rs", "src/session.rs", false),
        ("src/github/tests.rs", "src/gitlab.rs", false),
        ("src/tests.rs", "src/ui/chrome.rs", false),
    ] {
        assert_eq!(covers(test, source), expected, "{test} for {source}");
    }
}
#[test]
fn dependencies_are_modules_not_imported_names() {
    assert_eq!(dependency("serde::Deserialize").as_deref(), Some("serde"));
    assert_eq!(
        dependency("crate::store::Store").as_deref(),
        Some("crate::store")
    );
    assert_eq!(dependency("react").as_deref(), Some("react"));
    for relative in ["std::sync", "core::fmt", "super", "self::parse"] {
        assert_eq!(dependency(relative), None, "{relative}");
    }
}
#[tokio::test]
async fn test_gap_debt_and_coupling_ignore_test_code_and_new_modules_imports() {
    let mut ctx = fixture();
    let diff = "diff --git a/src/store.rs b/src/store.rs\n--- a/src/store.rs\n+++ b/src/store.rs\n@@ -1,0 +2,2 @@\n+use crate::cache::Cache;\n+use std::sync::Arc;\n@@ -2,0 +5,3 @@\n+fn save() { write().unwrap(); }\n+#[cfg(test)]\n+mod tests { use wiremock::Mock; #[test] fn saves() { save().unwrap(); } }\ndiff --git a/src/report.rs b/src/report.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/report.rs\n@@ -0,0 +1,2 @@\n+use serde::Serialize;\n+fn render() { parse().unwrap(); }\n";
    ctx.files = mrde_insights::diff::parse(diff);
    ctx.contents = BTreeMap::from([
        (
            "src/store.rs".to_string(),
            (
                Some("use crate::model;\nfn load() {}\n".to_string()),
                Some("use crate::model;\nuse crate::cache::Cache;\nuse std::sync::Arc;\nfn load() {}\nfn save() { write().unwrap(); }\n#[cfg(test)]\nmod tests { use wiremock::Mock; #[test] fn saves() { save().unwrap(); } }\n".to_string()),
            ),
        ),
        (
            "src/report.rs".to_string(),
            (
                None,
                Some("use serde::Serialize;\nfn render() { parse().unwrap(); }\n".to_string()),
            ),
        ),
    ]);
    let gap = TestGapScorer.measure(&ctx).await.unwrap();
    assert_eq!(gap.value, 2., "only the new file's two lines lack tests");
    assert_eq!(gap.evidence[0].path, "src/report.rs");
    assert_eq!(gap.evidence[1].detail, "Tests changed in the same file");
    let debt = DebtScorer.measure(&ctx).await.unwrap();
    let lines: Vec<_> = debt
        .evidence
        .iter()
        .map(|e| (e.path.as_str(), e.line))
        .collect();
    assert_eq!(
        lines,
        [("src/store.rs", Some(5)), ("src/report.rs", Some(2))]
    );
    let coupling = CouplingScorer.measure(&ctx).await.unwrap();
    assert_eq!(coupling.value, 1.);
    assert_eq!(
        coupling.evidence[0].detail,
        "New import target: crate::cache"
    );
}
#[tokio::test]
async fn generated_and_excluded_files_count_toward_no_axis() {
    let mut ctx = fixture();
    ctx.stats.files.push(mrde_core::models::FileStat {
        path: "Cargo.lock".into(),
        added: 900,
        removed: 300,
        binary: false,
    });
    ctx.excluded = vec!["Cargo.lock".into()];
    let size = DiffScorer.measure(&ctx).await.unwrap();
    assert_eq!(size.value, 5.);
    assert!(
        size.method
            .contains("1 generated or excluded file not counted")
    );
    assert_eq!(FileScorer.measure(&ctx).await.unwrap().value, 2.);
}
fn xml(ccn: u32) -> String {
    format!(
        r#"<cppncss><measure type="Function"><item name="login(...) at ./src/auth.rs:1"><value>1</value><value>4</value><value>{ccn}</value></item></measure><measure type="File"><item name="./src/auth.rs"><value>1</value><value>4</value><value>{ccn}</value><value>1</value></item><item name="./tests/auth.rs"><value>0</value></item></measure></cppncss>"#
    )
}
#[test]
fn complexity_parser_ignores_summary_totals_and_rejects_invalid_reports() {
    let report = xml(5);
    let totals = reports::complexity(&report, Path::new("/workspace")).unwrap();
    assert_eq!(totals["src/auth.rs"], 5.);
    assert!(reports::complexity("<cppncss/>", Path::new("/workspace")).is_err());
}
#[test]
fn diagnostic_formats_normalize_paths_and_ignore_line_shifts() {
    let root = Path::new("/workspace");
    let mut command = Command {
        id: "lint".into(),
        format: "json".into(),
        ..Default::default()
    };
    let a = reports::diagnostics(
        &command,
        r#"[{"path":"/workspace/src/a.rs","line":4,"rule":"x","message":"unused"}]"#,
        root,
    )
    .unwrap();
    let b = reports::diagnostics(
        &command,
        r#"[{"path":"src/a.rs","line":44,"rule":"x","message":"unused"}]"#,
        root,
    )
    .unwrap();
    assert_eq!(a[0].key(), b[0].key());
    command.format = "eslint".into();
    assert_eq!(reports::diagnostics(&command,r#"[{"filePath":"src/a.ts","messages":[{"line":2,"ruleId":"unused","message":"unused variable","severity":1}]}]"#,root).unwrap().len(),1);
    command.format = "clippy".into();
    assert_eq!(reports::diagnostics(&command,r#"{"reason":"compiler-message","message":{"level":"warning","message":"unused","spans":[{"is_primary":true,"file_name":"src/a.rs","line_start":3}],"code":{"code":"unused"}}}"#,root).unwrap().len(),1);
    command.format = "regex".into();
    command.warning_regex = r"(?m)^(?P<path>[^:]+):(?P<line>\d+): (?P<message>.+)$".into();
    assert_eq!(
        reports::diagnostics(&command, "src/a.rs:2: warning", root).unwrap()[0].line,
        2
    );
}
fn git(root: &Path, args: &[&str]) -> String {
    let out = Process::new("git")
        .current_dir(root)
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
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn write(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}
#[tokio::test]
async fn two_commit_repository_measures_all_axes_without_modifying_dirty_worktree() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "developer@example.com"]);
    git(root, &["config", "user.name", "Developer"]);
    write(root, "src/auth.rs", "fn login() {}\n");
    write(root, "complexity.xml", &xml(1));
    write(
        root,
        "diagnostics.json",
        r#"[{"path":"src/auth.rs","line":1,"rule":"old","message":"old"}]"#,
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "Initial fixture"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    write(
        root,
        "src/auth.rs",
        "use crate::session;\nfn login() { session::run().unwrap(); }\n// TODO expiry\n",
    );
    write(root, "tests/auth.rs", "#[test] fn login_test() {}\n");
    write(root, "complexity.xml", &xml(5));
    write(
        root,
        "diagnostics.json",
        r#"[{"path":"src/auth.rs","line":2,"rule":"old","message":"old"},{"path":"src/auth.rs","line":3,"rule":"new","message":"new"}]"#,
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "Change authentication"]);
    let head = git(root, &["rev-parse", "HEAD"]);
    write(root, "src/auth.rs", "uncommitted changes must stay\n");
    let mut config = RiskConfig::default();
    config.complexity.command = vec!["/bin/sh".into(), "-c".into(), "cat complexity.xml".into()];
    config.lint = vec![Command {
        id: "lint".into(),
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            "printf broken > complexity.xml; cat diagnostics.json".into(),
        ],
        format: "json".into(),
        approved: true,
        ..Default::default()
    }];
    let report = analyze(context(root, &base, &head, config.clone()).await.unwrap()).await;
    assert_eq!(report.profile.available(), 8, "{:?}", report.profile.axes);
    assert_eq!(
        report.profile.axes[Axis::Complexity.index()]
            .raw
            .as_ref()
            .unwrap()
            .value,
        4.
    );
    assert_eq!(
        report.profile.axes[Axis::Lint.index()]
            .raw
            .as_ref()
            .unwrap()
            .value,
        1.
    );
    assert_eq!(
        std::fs::read_to_string(root.join("src/auth.rs")).unwrap(),
        "uncommitted changes must stay\n"
    );
    assert!(git(root, &["diff", "--cached"]).is_empty());
    let history = mrde_git::risk::file_context(root, &base, &head, "src/auth.rs")
        .await
        .unwrap();
    assert_eq!(history.commits.len(), 2);
    assert_eq!(
        history.commits.iter().map(|c| c.in_mr).collect::<Vec<_>>(),
        [true, false],
        "the MR commit is marked, the earlier one is history"
    );
    assert_eq!(history.authors[0].1, 100.);
    let mut incomplete = config.clone();
    incomplete.complexity.command = vec![
        "/bin/sh".into(),
        "-c".into(),
        "sed 's@tests/auth.rs@tests/omitted.rs@g' complexity.xml".into(),
    ];
    let incomplete = analyze(context(root, &base, &head, incomplete).await.unwrap()).await;
    assert!(
        incomplete.profile.axes[Axis::Complexity.index()]
            .unavailable
            .as_ref()
            .unwrap()
            .contains("did not report")
    );
    assert_eq!(incomplete.profile.available(), 7);
    config.timeout_seconds = 1;
    config.complexity.command = vec!["/bin/sh".into(), "-c".into(), "sleep 3".into()];
    let partial = analyze(context(root, &base, &head, config).await.unwrap()).await;
    assert!(
        partial.profile.axes[Axis::Complexity.index()]
            .score
            .is_none()
    );
    assert_eq!(partial.profile.available(), 7);
}

#[tokio::test]
async fn detects_only_changed_packages_and_never_executes_manifest_scripts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "developer@example.com"]);
    git(root, &["config", "user.name", "Developer"]);
    write(
        root,
        "mobile/pubspec.yaml",
        "name: fixture\ndependencies:\n  flutter:\n    sdk: flutter\n",
    );
    write(
        root,
        "web/package.json",
        r#"{"devDependencies":{"eslint":"9.0.0"},"scripts":{"lint":"touch sentinel"}}"#,
    );
    write(root, "server/go.mod", "module example.invalid/server\n");
    write(
        root,
        "unrelated/Cargo.toml",
        "[package]\nname='unrelated'\nversion='0.1.0'\n",
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "Manifests"]);
    let sha = git(root, &["rev-parse", "HEAD"]);
    let changed = [
        "mobile/lib/screen.dart",
        "web/src/index.ts",
        "server/main.go",
    ]
    .map(str::to_string);
    let commands = detect::lint_commands(root, &sha, &changed).await.unwrap();
    assert_eq!(commands.len(), 3);
    assert!(commands.iter().all(|c| !c.approved));
    assert_eq!(
        commands
            .iter()
            .map(|c| c.directory.as_str())
            .collect::<Vec<_>>(),
        ["mobile", "server", "web"]
    );
    assert_eq!(commands[0].preparation, Some(config::Preparation::Flutter));
    assert_eq!(commands[2].command[0], "node");
    assert!(!root.join("sentinel").exists());
    // Detection reads the reviewed commit, not edits to the live worktree.
    write(root, "web/package.json", "not JSON");
    assert_eq!(
        detect::lint_commands(root, &sha, &changed).await.unwrap()[2].key(),
        commands[2].key()
    );
    assert!(
        detect::lint_commands(root, &sha, &["README.md".into()])
            .await
            .unwrap()
            .is_empty()
    );
}

#[test]
fn parses_detected_dart_go_and_python_diagnostics() {
    let root = Path::new("/snapshot");
    let mut command = Command {
        format: "dart".into(),
        ..Command::default()
    };
    let diagnostics = reports::diagnostics(&command, "WARNING|STATIC_WARNING|unused_local_variable|/snapshot/lib/main.dart|3|4|2|Unused variable\n", root).unwrap();
    assert_eq!(diagnostics[0].path, "lib/main.dart");
    assert_eq!(diagnostics[0].line, 3);
    assert!(reports::diagnostics(&command, "Analyzer failed", root).is_err());
    command.format = "go_vet".into();
    let diagnostics = reports::diagnostics(&command, "# example.invalid/app\n{\"example.invalid/app\":{\"printf\":[{\"posn\":\"/snapshot/main.go:8:2\",\"message\":\"format mismatch\"}]}}\n{}", root).unwrap();
    assert_eq!(diagnostics[0].path, "main.go");
    assert_eq!(diagnostics[0].rule, "printf");
    command.format = "ruff".into();
    let diagnostics = reports::diagnostics(&command, r#"[{"filename":"/snapshot/main.py","location":{"row":2},"code":"F401","message":"Unused import"}]"#, root).unwrap();
    assert_eq!(diagnostics[0].path, "main.py");
    assert_eq!(diagnostics[0].line, 2);
}

#[tokio::test]
async fn missing_complexity_tool_leaves_other_axes_available_with_setup_guidance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "developer@example.com"]);
    git(root, &["config", "user.name", "Developer"]);
    write(root, "main.py", "def value():\n    return 1\n");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "Before"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    write(
        root,
        "main.py",
        "def value(x):\n    if x:\n        return 1\n    return 2\n",
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "After"]);
    let head = git(root, &["rev-parse", "HEAD"]);
    let mut config = RiskConfig::default();
    config.complexity.command[0] = "mrde-missing-complexity-fixture".into();
    let ctx = context(root, &base, &head, config).await.unwrap();
    assert_eq!(ctx.config.lint_commands().len(), 1);
    assert!(!ctx.config.lint_commands()[0].approved);
    let report = analyze(ctx).await;
    let complexity = &report.profile.axes[Axis::Complexity.index()];
    assert!(
        complexity
            .unavailable
            .as_ref()
            .unwrap()
            .contains("Detect and install tools")
    );
    assert_eq!(report.pending_commands.len(), 1);
    assert_eq!(report.pending_commands[0].id, "ruff");
    assert!(report.profile.axes[Axis::DiffSize.index()].score.is_some());
    assert!(
        report.profile.axes[Axis::Lint.index()]
            .unavailable
            .as_ref()
            .unwrap()
            .contains("approval")
    );
}

#[tokio::test]
async fn go_vet_reports_on_either_stream_contribute_to_the_warning_delta() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "developer@example.com"]);
    git(root, &["config", "user.name", "Developer"]);
    write(root, "main.go", "package main\nfunc main() {}\n");
    write(root, "vet.json", "{}");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "Before"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    write(
        root,
        "main.go",
        "package main\nfunc main() {}\n// TODO review\n",
    );
    write(
        root,
        "vet.json",
        r#"{"fixture":{"printf":[{"posn":"main.go:2:1","message":"format mismatch"}]}}"#,
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "After"]);
    let head = git(root, &["rev-parse", "HEAD"]);
    for script in ["cat vet.json", "cat vet.json >&2"] {
        let config = RiskConfig {
            lint: vec![Command {
                id: "vet-fixture".into(),
                command: vec!["/bin/sh".into(), "-c".into(), script.into()],
                format: "go_vet".into(),
                approved: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let ctx = context(root, &base, &head, config).await.unwrap();
        let raw = LintScorer.measure(&ctx).await.unwrap();
        assert_eq!(raw.value, 1.);
        assert_eq!(raw.evidence[0].path, "main.go");
    }
}

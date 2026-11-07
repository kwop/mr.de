use mrde_core::risk::Axis;
use mrde_radar::{ScanReport, ScanRequest, scan};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Repo {
    root: PathBuf,
    config: PathBuf,
    _temp: tempfile::TempDir,
}
impl Repo {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo with spaces");
        let config = temp.path().join("settings");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&config).unwrap();
        let repo = Self {
            root,
            config,
            _temp: temp,
        };
        repo.git(&["init", "-q", "--template=", "--initial-branch=main"]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        repo.git(&["config", "user.name", "Fixture"]);
        repo.write("src/auth.rs", "pub fn login() {}\n");
        repo.write("src/remove.rs", "pub fn obsolete() {}\n");
        repo.write(".gitignore", "*.log\ncoverage/\n");
        repo.commit();
        repo
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn git(&self, args: &[&str]) -> String {
        let result = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap().trim().into()
    }
    fn commit(&self) {
        self.git(&["add", "."]);
        self.git(&[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "Fixture",
        ]);
    }
    fn request(&self) -> ScanRequest {
        ScanRequest {
            repository: self.root.clone(),
            base: "HEAD".into(),
            head: None,
            config_dir: self.config.clone(),
            project: "local".into(),
            run_checks: false,
            prepare_tools: false,
            tool_cache: Some(self.config.join("tools")),
            approved_commands: HashSet::new(),
        }
    }
    fn cli(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_mrde-radar"))
            .args([
                "--repo",
                self.root.to_str().unwrap(),
                "--config-dir",
                self.config.to_str().unwrap(),
            ])
            .args(args)
            .output()
            .unwrap()
    }
}
fn paths(report: &ScanReport) -> Vec<&str> {
    report
        .report
        .stats
        .files
        .iter()
        .map(|f| f.path.as_str())
        .collect()
}
fn object_files(path: &Path) -> usize {
    fs::read_dir(path)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            if p.is_dir() { object_files(&p) } else { 1 }
        })
        .sum()
}

#[tokio::test]
async fn working_tree_includes_staged_unstaged_new_and_deleted_without_changing_git() {
    let repo = Repo::new();
    repo.write("src/auth.rs", "pub fn login() {}\n// TODO staged\n");
    repo.git(&["add", "src/auth.rs"]);
    repo.write(
        "src/auth.rs",
        "pub fn login() {}\n// TODO staged\n// FIXME unstaged\n",
    );
    repo.write("src/new [file].rs", "pub fn added() {}\n");
    repo.git(&["rm", "src/remove.rs"]);
    repo.write("private.log", "ignored\n");
    repo.write(".git/info/exclude", "local-only/\n");
    repo.write("local-only/secret.rs", "ignored locally\n");
    let index = fs::read(repo.root.join(".git/index")).unwrap();
    let status = repo.git(&["status", "--porcelain", "--untracked-files=all"]);
    let head = repo.git(&["rev-parse", "HEAD"]);
    let objects = object_files(&repo.root.join(".git/objects"));
    let report = scan(repo.request()).await.unwrap();
    assert_eq!(
        paths(&report),
        vec!["src/auth.rs", "src/new [file].rs", "src/remove.rs"]
    );
    assert_eq!(
        report.report.profile.axes[Axis::Debt.index()]
            .raw
            .as_ref()
            .unwrap()
            .value,
        2.
    );
    assert_eq!(
        report.report.profile.axes[Axis::DiffSize.index()]
            .raw
            .as_ref()
            .unwrap()
            .value,
        4.
    );
    assert_ne!(report.report.profile.head_sha, head);
    assert_eq!(report.input.source_head, head);
    assert_eq!(index, fs::read(repo.root.join(".git/index")).unwrap());
    assert_eq!(
        status,
        repo.git(&["status", "--porcelain", "--untracked-files=all"])
    );
    assert_eq!(head, repo.git(&["rev-parse", "HEAD"]));
    assert_eq!(objects, object_files(&repo.root.join(".git/objects")));
    assert!(repo.root.join("local-only/secret.rs").exists());
    let again = scan(repo.request()).await.unwrap();
    assert_eq!(
        again.report.profile.head_sha, report.report.profile.head_sha,
        "snapshot identity is stable for identical files"
    );
}

#[tokio::test]
async fn committed_scan_matches_the_shared_engine_and_ignores_current_edits() {
    let repo = Repo::new();
    let base = repo.git(&["rev-parse", "HEAD"]);
    repo.write("src/auth.rs", "pub fn login() {}\n// TODO validate\n");
    repo.commit();
    let head = repo.git(&["rev-parse", "HEAD"]);
    repo.write("src/dirty.rs", "// FIXME ignored in commit mode\n");
    repo.write("risk.toml", "[normalization]\ndiff_size = [[0,0],[1,10]]\n");
    let mut request = repo.request();
    request.base = base.clone();
    request.head = Some("HEAD".into());
    let report = scan(request).await.unwrap();
    let mut context = mrde_risk::context(&repo.root, &base, &head, Default::default())
        .await
        .unwrap();
    // Match the CLI's default external-check opt-out, even if the host has Lizard.
    for command in context.config.commands_mut() {
        command.approved = false;
    }
    let direct = mrde_risk::analyze(context).await;
    for axis in Axis::ALL {
        assert_eq!(
            report.report.profile.axes[axis.index()].score,
            direct.profile.axes[axis.index()].score,
            "{axis:?}"
        );
    }
    assert_eq!(paths(&report), vec!["src/auth.rs"]);
    assert_eq!(report.report.profile.head_sha, head);
}

#[tokio::test]
async fn a_linked_worktree_is_scanned_in_its_own_branch() {
    let repo = Repo::new();
    let linked = repo._temp.path().join("linked tree");
    repo.git(&["worktree", "add", "-qb", "topic", linked.to_str().unwrap()]);
    fs::write(linked.join("src/auth.rs"), "// TODO linked change\n").unwrap();
    let mut request = repo.request();
    request.repository = linked;
    let report = scan(request).await.unwrap();
    assert_eq!(paths(&report), vec!["src/auth.rs"]);
    assert_eq!(
        fs::read_to_string(repo.root.join("src/auth.rs")).unwrap(),
        "pub fn login() {}\n"
    );
}

#[tokio::test]
async fn source_clean_filters_are_never_executed() {
    let repo = Repo::new();
    repo.write(".gitattributes", "*.rs filter=probe\n");
    repo.git(&["config", "filter.probe.clean", "touch filter-was-run; cat"]);
    repo.write("src/auth.rs", "pub fn login() {}\n// TODO inspect\n");
    scan(repo.request()).await.unwrap();
    assert!(!repo.root.join("filter-was-run").exists());
}

#[tokio::test]
async fn proposed_checks_require_an_explicit_switch_and_exact_fingerprint() {
    let repo = Repo::new();
    repo.write("src/auth.rs", "pub fn login() {}\n// TODO inspect\n");
    repo.write("risk.toml",r#"[[lint]]
id = "fixture-lint"
command = ["/bin/sh", "-c", "touch check-ran; if grep -q TODO src/auth.rs; then printf '[{\"path\":\"src/auth.rs\",\"line\":2,\"message\":\"new warning\"}]'; else printf '[]'; fi"]
format = "json"
"#);
    let report = scan(repo.request()).await.unwrap();
    assert!(
        report.report.profile.axes[Axis::Lint.index()]
            .score
            .is_none()
    );
    let proposed = report
        .report
        .pending_commands
        .iter()
        .find(|c| c.id == "fixture-lint")
        .unwrap();
    let mut request = repo.request();
    request.run_checks = true;
    let still_proposed = scan(request.clone()).await.unwrap();
    assert!(
        still_proposed.report.profile.axes[Axis::Lint.index()]
            .score
            .is_none()
    );
    request.approved_commands.insert(proposed.key.clone());
    let approved = scan(request.clone()).await.unwrap();
    let lint = &approved.report.profile.axes[Axis::Lint.index()];
    assert!(lint.raw.is_some(), "{:?}", lint.unavailable);
    assert_eq!(lint.raw.as_ref().unwrap().value, 1.);
    assert!(
        !repo.root.join("check-ran").exists(),
        "checks run in disposable exports"
    );
    assert!(approved.report.pending_commands.is_empty());
    request.approved_commands = HashSet::from(["0".repeat(64)]);
    assert!(
        scan(request)
            .await
            .unwrap_err()
            .to_string()
            .contains("fingerprint")
    );
}

#[tokio::test]
async fn coverage_failures_leave_other_axes_available() {
    let repo = Repo::new();
    repo.write("risk.toml", "coverage_path = 'coverage/lcov.info'\n");
    repo.write("src/auth.rs", "pub fn login() {}\n// TODO inspect\n");
    repo.write("coverage/lcov.info", "not a coverage report");
    let report = scan(repo.request()).await.unwrap();
    assert!(
        report.report.profile.axes[Axis::TestGap.index()]
            .score
            .is_none()
    );
    assert!(
        report.report.profile.axes[Axis::DiffSize.index()]
            .score
            .is_some()
    );
    assert!(report.summary.partial);
    #[cfg(unix)]
    {
        fs::remove_file(repo.root.join("coverage/lcov.info")).unwrap();
        let external = repo._temp.path().join("outside.txt");
        fs::write(&external, "private").unwrap();
        std::os::unix::fs::symlink(external, repo.root.join("coverage/lcov.info")).unwrap();
        let report = scan(repo.request()).await.unwrap();
        assert!(report.warnings.iter().any(|w| w.contains("escapes")));
        assert!(
            report.report.profile.axes[Axis::DiffSize.index()]
                .score
                .is_some()
        );
    }
}

#[test]
fn cli_emits_only_versioned_json_and_reports_failures_with_exit_codes() {
    let repo = Repo::new();
    repo.write("src/auth.rs", "// TODO inspect\n");
    let output = repo.cli(&["--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: ScanReport = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.schema_version, 1);
    assert!(report.summary.partial);
    assert!(report.baseline.is_none());
    assert!(output.stderr.is_empty());
    let missing = repo.cli(&["--base", "missing-local-ref", "--json"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    for args in [
        vec!["--head", "HEAD", "--worktree"],
        vec!["--allow-command", "abc"],
        vec!["--base", "--json"],
        vec!["--format", "html"],
    ] {
        let invalid = repo.cli(&args);
        assert_eq!(invalid.status.code(), Some(2), "{args:?}");
        assert!(invalid.stdout.is_empty());
    }
}

#[test]
fn svg_is_valid_and_leaves_missing_axes_unplotted() {
    let repo = Repo::new();
    repo.write("src/auth.rs", "// TODO inspect\n");
    let output = repo.cli(&["--format", "svg"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let xml = String::from_utf8(output.stdout).unwrap();
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut points = 0;
    loop {
        match reader.read_event().unwrap() {
            quick_xml::events::Event::Empty(e) if e.name().as_ref() == b"circle" => points += 1,
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    let data = repo.cli(&["--json"]);
    let report: ScanReport = serde_json::from_slice(&data.stdout).unwrap();
    assert_eq!(points, report.summary.measured_axes);
    assert!(xml.contains("n/a"));
    assert!(xml.contains("Team median unavailable"));
}

#[tokio::test]
async fn absolute_coverage_paths_are_resolved_against_the_source_checkout() {
    let repo = Repo::new();
    repo.write("risk.toml", "coverage_path = 'coverage/lcov.info'\n");
    repo.write("src/auth.rs", "pub fn login() {\n    validate();\n}\n");
    repo.write(
        "coverage/lcov.info",
        &format!(
            "SF:{}/src/auth.rs\nDA:1,1\nDA:2,0\nDA:3,1\nend_of_record\n",
            repo.root.canonicalize().unwrap().display()
        ),
    );
    let report = scan(repo.request()).await.unwrap();
    let gap = &report.report.profile.axes[Axis::TestGap.index()];
    assert_eq!(
        gap.raw
            .as_ref()
            .expect("absolute coverage paths must match")
            .value,
        1.
    );
    assert_eq!(gap.raw.as_ref().unwrap().evidence[0].path, "src/auth.rs");
}

#[tokio::test]
async fn assumed_unchanged_files_are_not_silently_reported_as_clean() {
    let repo = Repo::new();
    repo.git(&["update-index", "--assume-unchanged", "src/auth.rs"]);
    repo.write("src/auth.rs", "// FIXME hidden change\n");
    let index = fs::read(repo.root.join(".git/index")).unwrap();
    let error = scan(repo.request()).await.unwrap_err();
    assert!(error.to_string().contains("assume-unchanged"));
    assert_eq!(index, fs::read(repo.root.join(".git/index")).unwrap());
    let mut request = repo.request();
    request.head = Some("HEAD".into());
    assert!(scan(request).await.is_ok());
}

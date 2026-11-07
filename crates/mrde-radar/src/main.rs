use anyhow::{Context, Result, bail, ensure};
use mrde_radar::{ScanRequest, render, scan};
use std::{collections::HashSet, path::PathBuf};

const HELP: &str = "MR.DE local risk radar

Usage: mrde-radar [OPTIONS]

  --repo PATH           Repository or a directory inside it (default: .)
  --base REF            Compare from the merge base with REF (default: HEAD)
  --head REF            Analyze this commit instead of current files
  --worktree            Analyze staged, unstaged and non-ignored new files (default)
  --project ID          risk.toml project override (default: local)
  --config-dir PATH     Configuration root (default: ~/.config/mrde)
  --run-checks          Enable trusted complexity/lint commands via the check executor
  --allow-command KEY   Approve an exact proposed fingerprint for this run; repeatable
  --format FORMAT      text (default), json or svg
  --json                Shortcut for --format json
  -h, --help            Show this help
  --version             Show the version

Scans never fetch Git, access credentials or launch Claude. Default scans run no external
checks. Missing axes are n/a; exit 0 means a report was produced, not that risk is low.
Exit 1: scan failure. Exit 2: invalid arguments. Working-tree snapshots leave the source
index, branches and files unchanged. Ignored files are omitted except configured coverage.
";

struct Options {
    request: ScanRequest,
    format: String,
}
fn parse(args: Vec<String>) -> Result<Options> {
    let mut request = ScanRequest {
        repository: ".".into(),
        base: "HEAD".into(),
        head: None,
        config_dir: mrde_core::config::config_dir()?,
        project: "local".into(),
        run_checks: false,
        approved_commands: HashSet::new(),
    };
    let mut format = "text".to_string();
    let mut worktree = false;
    let mut args = args.into_iter();
    let mut seen = HashSet::new();
    while let Some(flag) = args.next() {
        ensure!(
            flag == "--allow-command" || seen.insert(flag.clone()),
            "Repeated option: {flag}"
        );
        match flag.as_str() {
            "--worktree" => worktree = true,
            "--run-checks" => request.run_checks = true,
            "--json" => format = "json".into(),
            "--repo" | "--base" | "--head" | "--project" | "--config-dir" | "--allow-command"
            | "--format" => {
                let value = args
                    .next()
                    .with_context(|| format!("{flag} requires a value"))?;
                ensure!(
                    !value.is_empty() && !value.starts_with("--"),
                    "{flag} requires a value before the next option"
                );
                match flag.as_str() {
                    "--repo" => request.repository = PathBuf::from(value),
                    "--base" => request.base = value,
                    "--head" => request.head = Some(value),
                    "--project" => request.project = value,
                    "--config-dir" => request.config_dir = PathBuf::from(value),
                    "--allow-command" => {
                        request.approved_commands.insert(value);
                    }
                    "--format" => format = value,
                    _ => unreachable!(),
                }
            }
            _ => bail!("Unknown option {flag:?}; use --help"),
        }
    }
    ensure!(
        !(worktree && request.head.is_some()),
        "--worktree and --head are mutually exclusive"
    );
    ensure!(
        !(seen.contains("--format") && seen.contains("--json")),
        "Use either --format or --json"
    );
    ensure!(
        matches!(format.as_str(), "text" | "json" | "svg"),
        "--format must be text, json or svg"
    );
    ensure!(
        request.run_checks || request.approved_commands.is_empty(),
        "--allow-command requires --run-checks"
    );
    Ok(Options { request, format })
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| matches!(a.as_str(), "-h" | "--help")) {
        print!("{HELP}");
        return 0.into();
    }
    if args == ["--version"] {
        println!("mrde-radar {}", env!("CARGO_PKG_VERSION"));
        return 0.into();
    }
    let options = match parse(args) {
        Ok(value) => value,
        Err(e) => {
            eprintln!("{e:#}");
            return 2.into();
        }
    };
    match scan(options.request).await {
        Ok(report) => {
            let output = match options.format.as_str() {
                "json" => {
                    serde_json::to_string_pretty(&report).expect("finite serializable report")
                }
                "svg" => render::svg(&report),
                _ => render::text(&report),
            };
            use std::io::Write;
            match writeln!(std::io::stdout().lock(), "{output}") {
                Ok(()) => 0.into(),
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => 0.into(),
                Err(e) => {
                    eprintln!("Cannot write report: {e}");
                    1.into()
                }
            }
        }
        Err(e) => {
            eprintln!("Radar scan failed: {e:#}");
            1.into()
        }
    }
}

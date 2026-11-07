//! Shared bounded command executor. Only locally approved commands may run.
use crate::{Approval, Check, CheckResult, Status};
use anyhow::{Context, Result, ensure};
use std::{
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

pub struct Output {
    pub result: CheckResult,
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}
const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
async fn read(mut pipe: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = pipe.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        ensure!(
            output.len() + n <= limit,
            "Check output exceeded the configured limit"
        );
        output.extend_from_slice(&buf[..n]);
    }
    Ok(output)
}
#[cfg(unix)]
struct ProcessGroup(i32);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if self.0 > 0 {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(self.0),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
}
pub async fn run(check: &Check, directory: &Path, timeout: Duration) -> Result<Output> {
    ensure!(
        check.approval == Approval::Approved,
        "Check {} needs local approval",
        check.id
    );
    let executable = check
        .command
        .first()
        .context("A check command cannot be empty")?;
    let start = Instant::now();
    let mut command = Command::new(executable);
    command
        .args(&check.command[1..])
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("CARGO_NET_OFFLINE", "true")
        .env("npm_config_offline", "true")
        .env("PIP_NO_INDEX", "1");
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .with_context(|| format!("Cannot start check {} ({executable})", check.id))?;
    #[cfg(unix)]
    let _group = ProcessGroup(child.id().context("Missing child process ID")? as i32);
    let stdout = child.stdout.take().context("Missing check stdout")?;
    let stderr = child.stderr.take().context("Missing check stderr")?;
    let (status, stdout, stderr) = tokio::time::timeout(timeout, async {
        tokio::try_join!(
            async { Ok::<_, anyhow::Error>(child.wait().await?) },
            read(stdout, OUTPUT_LIMIT),
            read(stderr, 64 * 1024)
        )
    })
    .await
    .with_context(|| format!("Check {} timed out after {}s", check.id, timeout.as_secs()))??;
    Ok(Output {
        result: CheckResult {
            id: check.id.clone(),
            status: if status.success() {
                Status::Passed
            } else {
                Status::Failed
            },
            duration_ms: start.elapsed().as_millis() as u64,
            artifacts: vec![],
            details: serde_json::json!({"exit_code":status.code(),"stdout_bytes":stdout.len(),"stderr_bytes":stderr.len()}),
        },
        stdout: String::from_utf8(stdout).context("Check stdout is not UTF-8")?,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        code: status.code(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn check(script: &str) -> Check {
        Check {
            id: "fixture".into(),
            command: vec!["/bin/sh".into(), "-c".into(), script.into()],
            approval: Approval::Approved,
        }
    }
    #[tokio::test]
    async fn captures_nonzero_results_without_treating_warnings_as_process_errors() {
        let result = run(
            &check("printf 'warning'; exit 1"),
            Path::new("/tmp"),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(result.stdout, "warning");
        assert_eq!(result.code, Some(1));
        assert!(matches!(result.result.status, Status::Failed));
    }
    #[tokio::test]
    async fn proposed_commands_never_execute_and_timeouts_stop_descendants() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("sentinel");
        let mut proposed = check("touch sentinel");
        proposed.approval = Approval::Proposed;
        assert!(
            run(&proposed, temp.path(), Duration::from_secs(1))
                .await
                .is_err()
        );
        assert!(!file.exists());
        assert!(
            run(
                &check("(sleep 0.3; touch sentinel) & wait"),
                temp.path(),
                Duration::from_millis(30)
            )
            .await
            .is_err()
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!file.exists(), "timed-out descendants must not survive");
    }
}

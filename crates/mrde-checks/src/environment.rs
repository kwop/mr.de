//! Child-process paths for both terminal and Finder launches; never mutate global PATH.
use anyhow::{Context, Result};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Environment {
    pub directories: Vec<PathBuf>,
}
impl Default for Environment {
    fn default() -> Self {
        Self::from_paths(
            std::env::var_os("PATH").as_deref(),
            std::env::var_os("HOME").as_deref(),
        )
    }
}
impl Environment {
    pub fn from_paths(path: Option<&OsStr>, home: Option<&OsStr>) -> Self {
        let mut directories: Vec<_> = path
            .map(std::env::split_paths)
            .into_iter()
            .flatten()
            .filter(|p| p.is_absolute())
            .collect();
        if let Some(home) = home.map(PathBuf::from).filter(|p| p.is_absolute()) {
            directories.extend([
                home.join(".local/bin"),
                home.join(".cargo/bin"),
                home.join("fvm/default/bin"),
            ]);
        }
        directories
            .extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
        let mut seen = std::collections::HashSet::new();
        directories.retain(|p| seen.insert(p.clone()));
        Self { directories }
    }
    pub fn path(&self) -> Result<OsString> {
        std::env::join_paths(&self.directories).context("Invalid check executable search path")
    }
    pub fn prepend(&mut self, directory: PathBuf) {
        self.directories.retain(|p| p != &directory);
        self.directories.insert(0, directory);
    }
    pub fn find(&self, executable: &str, directory: &Path) -> Option<PathBuf> {
        let name = Path::new(executable);
        if name.components().count() != 1 || name.is_absolute() {
            let path = directory.join(name);
            return executable_file(&path).then_some(path);
        }
        self.directories
            .iter()
            .map(|p| p.join(name))
            .find(|p| executable_file(p))
    }
}
fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finder_paths_include_user_tools_without_changing_shell_precedence() {
        let env = Environment::from_paths(
            Some(OsStr::new("/custom/bin:/usr/bin:.:")),
            Some(OsStr::new("/users/test")),
        );
        assert_eq!(env.directories[0], Path::new("/custom/bin"));
        assert!(
            env.directories
                .contains(&PathBuf::from("/users/test/.local/bin"))
        );
        assert!(
            env.directories
                .contains(&PathBuf::from("/opt/homebrew/bin"))
        );
        assert!(env.directories.iter().all(|p| p.is_absolute()));
        assert_eq!(
            env.directories
                .iter()
                .filter(|p| **p == Path::new("/usr/bin"))
                .count(),
            1
        );
    }
}

#[cfg(all(test, unix))]
mod process_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[tokio::test]
    async fn child_uses_the_explicit_search_path_and_reports_missing_tools() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("tools");
        std::fs::create_dir(&bin).unwrap();
        let executable = bin.join("fixture-check");
        std::fs::write(&executable, "#!/bin/sh\nprintf 'found'").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = Environment {
            directories: vec![bin],
        };
        let mut check = crate::Check {
            id: "fixture".into(),
            command: vec!["fixture-check".into()],
            approval: crate::Approval::Approved,
        };
        let output = crate::runner::run_in(
            &check,
            temp.path(),
            std::time::Duration::from_secs(1),
            &env,
            crate::runner::Network::Offline,
        )
        .await
        .unwrap();
        assert_eq!(output.stdout, "found");
        check.command[0] = "absent".into();
        let error = crate::runner::run_in(
            &check,
            temp.path(),
            std::time::Duration::from_secs(1),
            &env,
            crate::runner::Network::Offline,
        )
        .await
        .err()
        .unwrap();
        assert!(error.to_string().contains("Detect and install tools"));
    }
}

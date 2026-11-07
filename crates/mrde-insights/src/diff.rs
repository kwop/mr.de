//! Parser for `git diff -U0 --find-renames` output.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub removed: Vec<String>,
    pub added: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// `None` for added files.
    pub old_path: Option<String>,
    /// `None` for deleted files.
    pub new_path: Option<String>,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    pub fn path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or_default()
    }
    pub fn added(&self) -> u32 {
        self.hunks.iter().map(|h| h.added.len() as u32).sum()
    }
    pub fn removed(&self) -> u32 {
        self.hunks.iter().map(|h| h.removed.len() as u32).sum()
    }
    /// Added lines with their line numbers in the new file.
    pub fn added_lines(&self) -> impl Iterator<Item = (u32, &str)> {
        self.hunks.iter().flat_map(|h| {
            h.added
                .iter()
                .enumerate()
                .map(move |(i, line)| (h.new_start + i as u32, line.as_str()))
        })
    }
    /// Removed lines with their line numbers in the old file.
    pub fn removed_lines(&self) -> impl Iterator<Item = (u32, &str)> {
        self.hunks.iter().flat_map(|h| {
            h.removed
                .iter()
                .enumerate()
                .map(move |(i, line)| (h.old_start + i as u32, line.as_str()))
        })
    }
}

fn unquote(path: &str) -> String {
    let path = path.trim_end();
    match path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) {
        Some(inner) => inner.replace("\\\"", "\"").replace("\\\\", "\\"),
        None => path.to_string(),
    }
}

fn side(value: &str, prefix: &str) -> Option<String> {
    let value = unquote(value);
    if value == "/dev/null" {
        return None;
    }
    Some(
        value
            .strip_prefix(prefix)
            .map(str::to_string)
            .unwrap_or(value),
    )
}

/// `-a,b` or `-a` (count 1).
fn range(spec: &str) -> (u32, u32) {
    let spec = &spec[1..];
    match spec.split_once(',') {
        Some((start, count)) => (start.parse().unwrap_or(0), count.parse().unwrap_or(0)),
        None => (spec.parse().unwrap_or(0), 1),
    }
}

pub fn parse(diff: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut lines = diff.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // Paths from the header; later lines refine them (binary files have no ---/+++).
            let split = rest.rfind(" \"b/").or_else(|| rest.rfind(" b/"));
            let (old, new) = match split {
                Some(index) => (&rest[..index], &rest[index + 1..]),
                None => (rest, rest),
            };
            files.push(FileDiff {
                old_path: side(old.trim_matches('"'), "a/"),
                new_path: side(new.trim_matches('"'), "b/"),
                ..Default::default()
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("--- ") {
            file.old_path = side(rest, "a/");
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            file.new_path = side(rest, "b/");
        } else if let Some(rest) = line.strip_prefix("rename from ") {
            file.old_path = Some(unquote(rest));
        } else if let Some(rest) = line.strip_prefix("rename to ") {
            file.new_path = Some(unquote(rest));
        } else if line.starts_with("new file mode") {
            file.old_path = None;
        } else if line.starts_with("deleted file mode") {
            file.new_path = None;
        } else if line.starts_with("Binary files ") {
            file.binary = true;
        } else if let Some(header) = line.strip_prefix("@@ ") {
            let mut parts = header.split_whitespace();
            let (old_start, old_count) = parts.next().map(range).unwrap_or_default();
            let (new_start, new_count) = parts.next().map(range).unwrap_or_default();
            let mut hunk = Hunk {
                old_start,
                old_count,
                new_start,
                new_count,
                ..Default::default()
            };
            // Counts delimit the hunk, so content starting with "---" is never a header.
            while (hunk.removed.len() as u32) < old_count || (hunk.added.len() as u32) < new_count {
                let Some(content) = lines.next() else { break };
                if let Some(text) = content.strip_prefix('-') {
                    hunk.removed.push(text.to_string());
                } else if let Some(text) = content.strip_prefix('+') {
                    hunk.added.push(text.to_string());
                } else if content.starts_with('\\') {
                    continue;
                } else {
                    break;
                }
            }
            while lines.peek().is_some_and(|l| l.starts_with('\\')) {
                lines.next();
            }
            file.hunks.push(hunk);
        }
    }
    // `diff --git a/x b/x` without ---/+++ (mode-only or pure rename): fall back to rename data.
    files.retain(|f| f.old_path.is_some() || f.new_path.is_some());
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    const DIFF: &str = "diff --git a/src/a.go b/src/a.go
index 1..2 100644
--- a/src/a.go
+++ b/src/a.go
@@ -3,2 +3 @@ func A() {
--- old comment
-x := 1
+x := 2
@@ -10,0 +10,2 @@
+y := 3
+z := 4
diff --git a/old.ts b/new.ts
similarity index 100%
rename from old.ts
rename to new.ts
diff --git a/logo.png b/logo.png
new file mode 100644
index 0000000..1
Binary files /dev/null and b/logo.png differ
diff --git a/gone.sql b/gone.sql
deleted file mode 100644
--- a/gone.sql
+++ /dev/null
@@ -1 +0,0 @@
-DROP TABLE t;
";
    #[test]
    fn parses_hunks_renames_binaries_and_deletions() {
        let files = parse(DIFF);
        assert_eq!(files.len(), 4);
        let a = &files[0];
        assert_eq!(a.path(), "src/a.go");
        assert_eq!(a.removed(), 2);
        assert_eq!(a.added(), 3);
        assert_eq!(a.hunks[0].removed[0], "-- old comment");
        assert_eq!(
            a.added_lines().collect::<Vec<_>>(),
            [(3, "x := 2"), (10, "y := 3"), (11, "z := 4")]
        );
        assert_eq!(a.removed_lines().next(), Some((3, "-- old comment")));
        assert_eq!(files[1].old_path.as_deref(), Some("old.ts"));
        assert_eq!(files[1].new_path.as_deref(), Some("new.ts"));
        assert!(files[2].binary && files[2].old_path.is_none());
        assert!(files[3].new_path.is_none());
        assert_eq!(files[3].removed_lines().next(), Some((1, "DROP TABLE t;")));
    }
}

//! Risky patterns on added lines only, with the new file for loop context.
use crate::{
    diff::FileDiff,
    lang::{self, Lang},
};
use mrde_core::insights::{Level, PatternFinding};
use regex::Regex;
use std::sync::LazyLock;

struct Rule {
    id: &'static str,
    langs: &'static [Lang],
    level: Level,
    title: &'static str,
    advice: &'static str,
    pattern: Regex,
    /// Only reported inside a loop body.
    in_loop: bool,
    /// Not reported in test files.
    skip_tests: bool,
}

#[allow(clippy::too_many_arguments)]
fn rule(
    id: &'static str,
    langs: &'static [Lang],
    level: Level,
    title: &'static str,
    advice: &'static str,
    pattern: &str,
    in_loop: bool,
    skip_tests: bool,
) -> Rule {
    Rule {
        id,
        langs,
        level,
        title,
        advice,
        pattern: Regex::new(pattern).expect("valid pattern rule"),
        in_loop,
        skip_tests,
    }
}

const ALL: &[Lang] = &[Lang::Go, Lang::Script, Lang::Rust, Lang::Python, Lang::Sql];
const CODE: &[Lang] = &[Lang::Go, Lang::Script, Lang::Rust, Lang::Python];

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        rule(
            "query-in-loop",
            CODE,
            Level::High,
            "Database or network call inside a loop",
            "Batch the operation (bulk insert, IN query, Promise.all) to avoid N round trips.",
            r"\.(?:Exec|ExecContext|Query|QueryRow|QueryContext|QueryRowContext|Insert|Create|Save|Update|Delete|Find|First|Get|Do|execute|query|fetchone|save|create)\s*\(|\bfetch\s*\(|\baxios\.\w+\s*\(|\bhttp\.(?:Get|Post)\s*\(",
            true,
            true,
        ),
        rule(
            "await-in-loop",
            &[Lang::Script, Lang::Python],
            Level::Medium,
            "Sequential await inside a loop",
            "Requests run one after another; collect promises and await them together if order does not matter.",
            r"\bawait\b",
            true,
            true,
        ),
        rule(
            "ignored-error",
            &[Lang::Go],
            Level::Medium,
            "Error ignored",
            "Handle or log the error; failures here are silent.",
            r"^\s*_\s*(?:,\s*_\s*)?=\s*[\w.]+\(|^\s*_\s*=\s*\w+\.\w+",
            false,
            true,
        ),
        rule(
            "empty-catch",
            &[Lang::Script],
            Level::Medium,
            "Empty catch block",
            "Errors are swallowed; at least log them.",
            r"catch\s*(?:\([^)]*\))?\s*\{\s*\}",
            false,
            false,
        ),
        rule(
            "panic",
            &[Lang::Go],
            Level::Medium,
            "panic in non-test code",
            "Return an error so callers can recover.",
            r"\bpanic\(",
            false,
            true,
        ),
        rule(
            "unwrap",
            &[Lang::Rust],
            Level::Medium,
            "unwrap/expect in non-test code",
            "Propagate the error with ? or handle it.",
            r"\.(?:unwrap|expect)\(",
            false,
            true,
        ),
        rule(
            "unsafe",
            &[Lang::Rust],
            Level::High,
            "unsafe block",
            "Check the invariants this code relies on.",
            r"\bunsafe\s*\{",
            false,
            false,
        ),
        rule(
            "html-injection",
            &[Lang::Script],
            Level::High,
            "Raw HTML injection",
            "Sanitize the content or render it as text.",
            r"dangerouslySetInnerHTML|\.innerHTML\s*=|v-html",
            false,
            false,
        ),
        rule(
            "eval",
            &[Lang::Script, Lang::Python],
            Level::High,
            "Dynamic code evaluation",
            "Avoid eval; parse the data instead.",
            r"\beval\s*\(|new Function\s*\(",
            false,
            false,
        ),
        rule(
            "secret",
            ALL,
            Level::High,
            "Possible hardcoded secret",
            "Load secrets from the environment or a secret store.",
            r#"(?i)\b(?:password|passwd|secret|api[_-]?key|token)\b\s*(?::=|=|:)\s*["'][^"'\s]{8,}["']"#,
            false,
            true,
        ),
        rule(
            "any-type",
            &[Lang::Script],
            Level::Low,
            "Type safety bypassed",
            "Prefer a precise type over any or @ts-ignore.",
            r":\s*any\b|\bas any\b|@ts-ignore|@ts-nocheck",
            false,
            true,
        ),
        rule(
            "debug-output",
            CODE,
            Level::Low,
            "Debug output left in code",
            "Remove it or use the project logger.",
            r"\bconsole\.log\(|\bfmt\.Print(?:ln|f)?\(|\bdbg!\(|^\s*print\(",
            false,
            true,
        ),
        rule(
            "select-star",
            &[Lang::Go, Lang::Script, Lang::Python, Lang::Sql, Lang::Rust],
            Level::Low,
            "SELECT *",
            "List the columns: schema changes then cannot break the reader silently.",
            r"(?i)\bselect\s+\*\s+from\b",
            false,
            true,
        ),
        rule(
            "todo",
            ALL,
            Level::Low,
            "TODO left in the change",
            "Track it in an issue or resolve it before merging.",
            r"\b(?:TODO|FIXME|XXX|HACK)\b",
            false,
            false,
        ),
        rule(
            "sleep",
            CODE,
            Level::Low,
            "Fixed sleep",
            "Waiting a fixed time is flaky; wait for a condition instead.",
            r"\btime\.Sleep\(|\bsetTimeout\(|thread::sleep|\btime\.sleep\(",
            false,
            true,
        ),
    ]
});

/// Line ranges (1-based, inclusive) of loop bodies in brace languages and Python.
fn loops(lang: Lang, source: &str) -> Vec<(u32, u32)> {
    let lines: Vec<&str> = source.lines().collect();
    let loop_start = Regex::new(match lang {
        Lang::Python => r"^\s*(?:async\s+)?(?:for|while)\b.*:\s*$",
        Lang::Rust => r"^\s*(?:for|while|loop)\b|\.for_each\(",
        _ => r"^\s*for\b|\.forEach\(|\.map\(\s*(?:async\s*)?\(?[\w\s,{}]*\)?\s*=>|^\s*while\b",
    })
    .expect("valid loop pattern");
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !loop_start.is_match(line) {
            continue;
        }
        if lang == Lang::Python {
            let indent = line.len() - line.trim_start().len();
            let mut end = index;
            for (i, next) in lines.iter().enumerate().skip(index + 1) {
                if !next.trim().is_empty() && next.len() - next.trim_start().len() <= indent {
                    break;
                }
                end = i;
            }
            out.push((index as u32 + 1, end as u32 + 1));
            continue;
        }
        let mut depth = 0i32;
        let mut opened = false;
        for (i, next) in lines.iter().enumerate().skip(index) {
            for c in next.chars() {
                match c {
                    '{' => {
                        depth += 1;
                        opened = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            if opened && depth <= 0 {
                out.push((index as u32 + 1, i as u32 + 1));
                break;
            }
            if i > index + 3 && !opened {
                break;
            }
        }
    }
    out
}

/// useEffect calls whose callback is not followed by a dependency array.
fn effects_without_deps(source: &str) -> Vec<u32> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !line.contains("useEffect(") {
            continue;
        }
        let mut depth = 0i32;
        for (i, next) in lines.iter().enumerate().skip(index) {
            let from = if i == index {
                next.find("useEffect(").unwrap_or(0) + "useEffect".len()
            } else {
                0
            };
            let mut closed = false;
            for (offset, c) in next[from..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            let call = &next[from..from + offset];
                            let before = lines[index..i].join("\n") + call;
                            let tail = before.trim_end();
                            if !tail.ends_with(']') {
                                out.push(index as u32 + 1);
                            }
                            closed = true;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if closed || i > index + 80 {
                break;
            }
        }
    }
    out
}

pub fn scan(file: &FileDiff, new: Option<&str>) -> Vec<PatternFinding> {
    let path = file.path();
    let lang = lang::of(path);
    if lang == Lang::Other || lang::is_generated(path) || file.binary {
        return Vec::new();
    }
    let test = lang::is_test(path);
    let loop_ranges = new.map(|s| loops(lang, s)).unwrap_or_default();
    let in_loop = |line: u32| {
        loop_ranges
            .iter()
            .any(|(start, end)| line > *start && line <= *end)
    };
    let mut out = Vec::new();
    for (line, text) in file.added_lines() {
        let trimmed = text.trim();
        if trimmed.starts_with("//") && !trimmed.contains("TODO") && !trimmed.contains("FIXME") {
            continue;
        }
        for rule in RULES.iter() {
            if !rule.langs.contains(&lang) || (rule.skip_tests && test) {
                continue;
            }
            if rule.in_loop && !in_loop(line) {
                continue;
            }
            if rule.pattern.is_match(text) {
                out.push(PatternFinding {
                    level: rule.level,
                    rule: rule.id.into(),
                    title: rule.title.into(),
                    path: path.into(),
                    line,
                    snippet: trimmed.chars().take(160).collect(),
                    advice: rule.advice.into(),
                });
            }
        }
    }
    if lang == Lang::Script
        && let Some(source) = new
    {
        let added: Vec<u32> = file.added_lines().map(|(l, _)| l).collect();
        for line in effects_without_deps(source) {
            if added.contains(&line) {
                out.push(PatternFinding {
                    level: Level::Medium,
                    rule: "effect-without-deps".into(),
                    title: "useEffect without a dependency array".into(),
                    path: path.into(),
                    line,
                    snippet: source
                        .lines()
                        .nth(line as usize - 1)
                        .unwrap_or_default()
                        .trim()
                        .into(),
                    advice: "The effect runs after every render; pass the values it depends on."
                        .into(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff;

    fn file(path: &str, source: &str) -> FileDiff {
        let added: Vec<String> = source.lines().map(|l| format!("+{l}")).collect();
        let raw = format!(
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n{}\n",
            added.len(),
            added.join("\n")
        );
        diff::parse(&raw).remove(0)
    }

    #[test]
    fn flags_loop_queries_ignored_errors_and_secrets_but_not_tests() {
        let go = "package x\n\nfunc Create(ds []time.Time) {\n\tfor _, d := range ds {\n\t\trepo.Insert(ctx, d)\n\t}\n\t_ = notifier.Publish(ctx, e)\n\tpassword := \"hunter2hunter2\"\n\trepo.Insert(ctx, last)\n}\n";
        let found = scan(&file("usecases/create_series.go", go), Some(go));
        let rules: Vec<(&str, u32)> = found.iter().map(|f| (f.rule.as_str(), f.line)).collect();
        assert!(rules.contains(&("query-in-loop", 5)));
        assert!(!rules.contains(&("query-in-loop", 9)), "outside the loop");
        assert!(rules.contains(&("ignored-error", 7)));
        assert!(rules.contains(&("secret", 8)));
        assert!(
            scan(&file("usecases/create_series_test.go", go), Some(go))
                .iter()
                .all(|f| f.rule != "query-in-loop")
        );
    }

    #[test]
    fn flags_effects_without_dependencies_only() {
        let tsx = "export function A() {\n  useEffect(() => {\n    load();\n  });\n  useEffect(() => {\n    save();\n  }, [value]);\n  return null;\n}\n";
        let found = scan(&file("ui/A.tsx", tsx), Some(tsx));
        let effects: Vec<u32> = found
            .iter()
            .filter(|f| f.rule == "effect-without-deps")
            .map(|f| f.line)
            .collect();
        assert_eq!(effects, [2]);
    }
}

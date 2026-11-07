//! Symbol-level diff: definitions added, removed, modified, or with a changed signature.
//! Line-based recognizers per language; spans end at the matching brace or, for Python,
//! where indentation returns to the definition level.
use crate::{
    diff::FileDiff,
    lang::{self, Lang},
};
use mrde_core::insights::{SymbolChange, SymbolChangeKind, SymbolKind};
use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub signature: String,
    /// 1-based, inclusive.
    pub start: u32,
    pub end: u32,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid symbol pattern")
}

static GO: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
    vec![
        (
            re(r"^func\s+\(\s*\w*\s*\*?(\w+)(?:\[[^\]]*\])?\s*\)\s*(\w+)\s*[\[(]"),
            SymbolKind::Method,
        ),
        (re(r"^func\s+(\w+)\s*[\[(]"), SymbolKind::Function),
        (re(r"^type\s+(\w+)\b"), SymbolKind::Type),
        (re(r"^(?:const|var)\s+(\w+)\b"), SymbolKind::Constant),
    ]
});
static SCRIPT: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
    vec![
        (
            re(r"^(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s*(\w+)"),
            SymbolKind::Function,
        ),
        (
            re(
                r"^(?:export\s+)?(?:const|let)\s+(\w+)\b[^=]*=\s*(?:async\s*)?(?:\(|function\b|\w+\s*=>|<[A-Za-z])",
            ),
            SymbolKind::Function,
        ),
        (
            re(r"^(?:export\s+)?(?:default\s+)?(?:abstract\s+)?class\s+(\w+)"),
            SymbolKind::Class,
        ),
        (
            re(r"^(?:export\s+)?(?:declare\s+)?(?:interface|type|enum)\s+(\w+)"),
            SymbolKind::Type,
        ),
        (
            re(r"^(?:export\s+)?const\s+([A-Z][A-Z0-9_]+)\b"),
            SymbolKind::Constant,
        ),
        (
            re(
                r"^\s{2,}(?:(?:public|private|protected|static|async|readonly|override)\s+)*(\w+)\s*(?:<[^>]*>)?\([^;]*\)\s*(?::\s*[^{=;]+)?\{\s*$",
            ),
            SymbolKind::Method,
        ),
    ]
});
static RUST: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
    vec![
        (
            re(
                r#"^(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?fn\s+(\w+)"#,
            ),
            SymbolKind::Function,
        ),
        (
            re(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|trait|type|union)\s+(\w+)"),
            SymbolKind::Type,
        ),
        (
            re(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+([A-Z][A-Z0-9_]*)\b"),
            SymbolKind::Constant,
        ),
    ]
});
static PYTHON: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
    vec![
        (re(r"^(\s*)(?:async\s+)?def\s+(\w+)"), SymbolKind::Function),
        (re(r"^class\s+(\w+)"), SymbolKind::Class),
    ]
});
const KEYWORDS: &[&str] = &[
    "if",
    "for",
    "while",
    "switch",
    "catch",
    "return",
    "function",
    "constructor",
];

/// Removes string literals and line comments so braces inside them are not counted.
fn code_only(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == '\\' {
                    chars.next();
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' | '`' => quote = Some(c),
                '/' if chars.peek() == Some(&'/') => break,
                _ => out.push(c),
            },
        }
    }
    out
}

fn brace_end(lines: &[&str], start: usize) -> usize {
    let mut depth = 0i32;
    let mut opened = false;
    for (index, line) in lines.iter().enumerate().skip(start) {
        let code = code_only(line);
        for c in code.chars() {
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
            return index;
        }
        // Bodiless declarations (`type X int`, `type A = B;`) end on their own line.
        if !opened && index >= start + 2 {
            return start;
        }
        if !opened
            && (code.trim_end().ends_with(';')
                || index == start
                    && !code.contains('(')
                    && !code.contains('=')
                    && !code.contains('{'))
        {
            return index;
        }
    }
    lines.len().saturating_sub(1)
}

fn indent_end(lines: &[&str], start: usize) -> usize {
    let indent = lines[start].len() - lines[start].trim_start().len();
    let mut end = start;
    for (index, line) in lines.iter().enumerate().skip(start + 1) {
        if line.trim().is_empty() {
            continue;
        }
        if line.len() - line.trim_start().len() <= indent {
            break;
        }
        end = index;
    }
    end
}

pub fn extract(lang: Lang, source: &str) -> Vec<Symbol> {
    let patterns = match lang {
        Lang::Go => &*GO,
        Lang::Script => &*SCRIPT,
        Lang::Rust => &*RUST,
        Lang::Python => &*PYTHON,
        _ => return Vec::new(),
    };
    let lines: Vec<&str> = source.lines().collect();
    let mut symbols = Vec::new();
    let mut skip_until = 0usize;
    for (index, line) in lines.iter().enumerate() {
        for (regex, kind) in patterns {
            let Some(captures) = regex.captures(line) else {
                continue;
            };
            let mut kind = *kind;
            let indented = line.starts_with(char::is_whitespace);
            let name = match (lang, kind) {
                (Lang::Go, SymbolKind::Method) => format!("{}.{}", &captures[1], &captures[2]),
                (Lang::Rust, SymbolKind::Function) | (Lang::Python, SymbolKind::Function) => {
                    if !captures[1].is_empty() {
                        kind = SymbolKind::Method;
                    }
                    captures[2].to_string()
                }
                _ => captures[1].to_string(),
            };
            // Script methods are only recognized inside a class body.
            if lang == Lang::Script
                && kind == SymbolKind::Method
                && (index > skip_until || KEYWORDS.contains(&name.as_str()))
            {
                continue;
            }
            if lang == Lang::Script && indented && kind != SymbolKind::Method {
                continue;
            }
            let end = if lang == Lang::Python {
                indent_end(&lines, index)
            } else {
                brace_end(&lines, index)
            };
            if kind == SymbolKind::Class {
                skip_until = end;
            }
            symbols.push(Symbol {
                name,
                kind,
                signature: line.split_whitespace().collect::<Vec<_>>().join(" "),
                start: index as u32 + 1,
                end: end as u32 + 1,
            });
            break;
        }
    }
    symbols
}

fn body(source: &str, symbol: &Symbol) -> String {
    source
        .lines()
        .skip(symbol.start as usize - 1)
        .take((symbol.end - symbol.start + 1) as usize)
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Compares the definitions of one file before and after the change.
pub fn compare(file: &FileDiff, old: Option<&str>, new: Option<&str>) -> Vec<SymbolChange> {
    let lang = lang::of(file.path());
    let old_symbols = old.map(|s| extract(lang, s)).unwrap_or_default();
    let new_symbols = new.map(|s| extract(lang, s)).unwrap_or_default();
    let count_added = |s: &Symbol| {
        file.added_lines()
            .filter(|(line, _)| (s.start..=s.end).contains(line))
            .count() as u32
    };
    let count_removed = |s: &Symbol| {
        file.removed_lines()
            .filter(|(line, _)| (s.start..=s.end).contains(line))
            .count() as u32
    };
    let mut changes = Vec::new();
    for symbol in &new_symbols {
        let previous = old_symbols
            .iter()
            .find(|o| o.name == symbol.name && o.kind == symbol.kind);
        let change = match previous {
            None => Some(SymbolChangeKind::Added),
            Some(previous) => {
                let (before, after) = (
                    body(old.unwrap_or_default(), previous),
                    body(new.unwrap_or_default(), symbol),
                );
                if before == after {
                    None
                } else if previous.signature != symbol.signature {
                    Some(SymbolChangeKind::SignatureChanged)
                } else {
                    Some(SymbolChangeKind::Modified)
                }
            }
        };
        let Some(change) = change else { continue };
        let added = count_added(symbol);
        let removed = previous.map(count_removed).unwrap_or(0);
        // A body change outside the diff is a moved symbol, not an edit.
        if change != SymbolChangeKind::Added && added == 0 && removed == 0 {
            continue;
        }
        changes.push(SymbolChange {
            path: file.path().to_string(),
            name: symbol.name.clone(),
            kind: symbol.kind,
            change,
            before: previous.map(|p| p.signature.clone()),
            after: Some(symbol.signature.clone()),
            added: if change == SymbolChangeKind::Added {
                symbol.end - symbol.start + 1
            } else {
                added
            },
            removed,
            line: symbol.start,
        });
    }
    for symbol in &old_symbols {
        if !new_symbols
            .iter()
            .any(|n| n.name == symbol.name && n.kind == symbol.kind)
        {
            changes.push(SymbolChange {
                path: file.path().to_string(),
                name: symbol.name.clone(),
                kind: symbol.kind,
                change: SymbolChangeKind::Removed,
                before: Some(symbol.signature.clone()),
                after: None,
                added: 0,
                removed: symbol.end - symbol.start + 1,
                line: symbol.start,
            });
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff;

    #[test]
    fn extracts_go_script_rust_and_python_definitions() {
        let go = "package x\n\ntype Series struct {\n\tID string\n}\n\nfunc (s *Series) Occurrences() []time.Time {\n\treturn nil\n}\n\nfunc CreateSeries(ctx context.Context) error {\n\tif x {\n\t\treturn nil\n\t}\n\treturn nil\n}\n";
        let names: Vec<_> = extract(Lang::Go, go)
            .into_iter()
            .map(|s| (s.name, s.start, s.end))
            .collect();
        assert_eq!(
            names,
            [
                ("Series".into(), 3, 5),
                ("Series.Occurrences".into(), 7, 9),
                ("CreateSeries".into(), 11, 16)
            ]
        );
        let ts = "import x from 'y';\nexport function SeriesDialog(props: Props) {\n  const s = \"}\";\n  return <div/>;\n}\nexport const useSeries = () => {\n  return 1;\n};\nexport type Rule = { every: number };\nclass Api {\n  async create(rule: Rule): Promise<void> {\n    if (x) {\n    }\n  }\n}\n";
        let names: Vec<_> = extract(Lang::Script, ts)
            .into_iter()
            .map(|s| (s.name, s.kind, s.end))
            .collect();
        assert_eq!(
            names,
            [
                ("SeriesDialog".into(), SymbolKind::Function, 5),
                ("useSeries".into(), SymbolKind::Function, 8),
                ("Rule".into(), SymbolKind::Type, 9),
                ("Api".into(), SymbolKind::Class, 15),
                ("create".into(), SymbolKind::Method, 14),
            ]
        );
        let rust = "pub struct A;\nimpl A {\n    pub fn run(&self) -> u8 {\n        1\n    }\n}\n";
        let symbols = extract(Lang::Rust, rust);
        assert_eq!(symbols[1].name, "run");
        assert_eq!(symbols[1].kind, SymbolKind::Method);
        let python =
            "class Job:\n    def run(self):\n        return 1\n\ndef helper():\n    pass\n";
        let names: Vec<_> = extract(Lang::Python, python)
            .into_iter()
            .map(|s| (s.name, s.end))
            .collect();
        assert_eq!(
            names,
            [("Job".into(), 3), ("run".into(), 3), ("helper".into(), 6)]
        );
    }

    #[test]
    fn classifies_added_removed_modified_and_signature_changes() {
        let old = "package x\n\nfunc Keep() {}\n\nfunc Validate(t T) error {\n\treturn check(t)\n}\n\nfunc Gone() {\n}\n\nfunc Create(ctx C) error {\n\treturn nil\n}\n";
        let new = "package x\n\nfunc Keep() {}\n\nfunc Validate(t T) error {\n\treturn checkAll(t)\n}\n\nfunc Create(ctx C, tx Tx) error {\n\treturn nil\n}\n\nfunc Added() {\n}\n";
        let raw = "diff --git a/x.go b/x.go\n--- a/x.go\n+++ b/x.go\n@@ -6 +6 @@\n-\treturn check(t)\n+\treturn checkAll(t)\n@@ -9,6 +9,6 @@\n-func Gone() {\n-}\n-\n-func Create(ctx C) error {\n-\treturn nil\n-}\n+func Create(ctx C, tx Tx) error {\n+\treturn nil\n+}\n+\n+func Added() {\n+}\n";
        let file = &diff::parse(raw)[0];
        let mut changes: Vec<_> = compare(file, Some(old), Some(new))
            .into_iter()
            .map(|c| (c.name, c.change))
            .collect();
        changes.sort();
        assert_eq!(
            changes,
            [
                ("Added".into(), SymbolChangeKind::Added),
                ("Create".into(), SymbolChangeKind::SignatureChanged),
                ("Gone".into(), SymbolChangeKind::Removed),
                ("Validate".into(), SymbolChangeKind::Modified),
            ]
        );
    }
}

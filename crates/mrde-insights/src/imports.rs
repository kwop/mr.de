//! Dependency graph between the files changed by the MR: imports, plus HTTP calls from
//! client code to endpoints declared in the MR.
use crate::lang::{self, Lang};
use mrde_core::insights::{FileGraph, GraphEdge};
use regex::Regex;
use std::{collections::BTreeSet, sync::LazyLock};

static SCRIPT_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:from\s*|import\s*\(\s*|require\(\s*|^\s*import\s+)['"]([^'"]+)['"]"#)
        .expect("valid import pattern")
});
static QUOTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""([^"]+)""#).expect("valid quote pattern"));
static RUST_USE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:pub\s+)?(?:use\s+crate::([\w:]+)|mod\s+(\w+)\s*;)")
        .expect("valid use pattern")
});
static PYTHON_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:from\s+([\w.]+)\s+import|import\s+([\w.]+))").expect("valid import pattern")
});
static PATH_LITERAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"['"`](/[A-Za-z0-9_\-./{}:$]+)['"`]"#).expect("valid path pattern")
});

/// Repository layout needed to resolve module imports.
#[derive(Debug, Default)]
pub struct Roots {
    /// (directory of go.mod, module path)
    pub go_modules: Vec<(String, String)>,
    /// Directories containing package.json.
    pub packages: Vec<String>,
    /// Directories containing Cargo.toml.
    pub crates: Vec<String>,
}

fn dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

fn join(base: &str, relative: &str) -> String {
    let mut parts: Vec<&str> = if base.is_empty() {
        vec![]
    } else {
        base.split('/').collect()
    };
    for part in relative.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn nearest<'a>(roots: &'a [String], path: &str) -> Option<&'a String> {
    roots
        .iter()
        .filter(|root| root.is_empty() || path.starts_with(&format!("{root}/")))
        .max_by_key(|root| root.len())
}

/// Candidate file paths an import of `file` may refer to.
fn targets(file: &str, source: &str, roots: &Roots) -> Vec<String> {
    let mut out = Vec::new();
    match lang::of(file) {
        Lang::Go => {
            let mut in_block = false;
            for line in source.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("import (") {
                    in_block = true;
                    continue;
                }
                if in_block && trimmed.starts_with(')') {
                    break;
                }
                if in_block || trimmed.starts_with("import ") {
                    for captures in QUOTED.captures_iter(trimmed) {
                        let import = &captures[1];
                        for (root, module) in &roots.go_modules {
                            if let Some(rest) = import.strip_prefix(&format!("{module}/")) {
                                // A Go import names a directory: every file in it.
                                out.push(format!("{}/", join(root, rest)));
                            }
                        }
                    }
                }
                if !in_block && trimmed.starts_with("func ") {
                    break;
                }
            }
        }
        Lang::Script => {
            for captures in SCRIPT_IMPORT.captures_iter(source) {
                let spec = &captures[1];
                let base = if spec.starts_with('.') {
                    join(dir(file), spec)
                } else if let Some(rest) =
                    spec.strip_prefix("@/").or_else(|| spec.strip_prefix("~/"))
                {
                    let root = nearest(&roots.packages, file).cloned().unwrap_or_default();
                    join(&join(&root, "src"), rest)
                } else {
                    continue;
                };
                for suffix in [
                    "",
                    ".ts",
                    ".tsx",
                    ".js",
                    ".jsx",
                    "/index.ts",
                    "/index.tsx",
                    "/index.js",
                ] {
                    out.push(format!("{base}{suffix}"));
                }
            }
        }
        Lang::Rust => {
            let root = nearest(&roots.crates, file).cloned().unwrap_or_default();
            for line in source.lines() {
                let Some(captures) = RUST_USE.captures(line) else {
                    continue;
                };
                if let Some(path) = captures.get(1) {
                    let segments: Vec<&str> = path.as_str().split("::").collect();
                    for take in 1..=segments.len() {
                        let module = segments[..take].join("/");
                        out.push(join(&root, &format!("src/{module}.rs")));
                        out.push(join(&root, &format!("src/{module}/mod.rs")));
                    }
                } else if let Some(module) = captures.get(2) {
                    let sibling = join(dir(file), module.as_str());
                    out.push(format!("{sibling}.rs"));
                    out.push(format!("{sibling}/mod.rs"));
                }
            }
        }
        Lang::Python => {
            for line in source.lines() {
                let Some(captures) = PYTHON_IMPORT.captures(line) else {
                    continue;
                };
                let module = captures
                    .get(1)
                    .or(captures.get(2))
                    .map(|m| m.as_str())
                    .unwrap_or_default();
                let path = module.trim_start_matches('.').replace('.', "/");
                let base = if module.starts_with('.') {
                    join(dir(file), &path)
                } else {
                    path
                };
                out.push(format!("{base}.py"));
                out.push(format!("{base}/__init__.py"));
            }
        }
        _ => {}
    }
    out
}

/// `/api/series/{id}`, `/api/series/:id`, and `/api/series/${id}` compare equal.
fn route_key(path: &str) -> String {
    path.trim_end_matches('/')
        .split('/')
        .map(|segment| {
            if segment.starts_with('{') || segment.starts_with(':') || segment.starts_with("${") {
                "*"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Builds the graph. `sources` holds (path, content at head); `routes` holds endpoints
/// declared by the MR as (path of declaring file, route).
pub fn graph(
    sources: &[(String, String)],
    roots: &Roots,
    routes: &[(String, String)],
) -> FileGraph {
    let paths: Vec<&str> = sources.iter().map(|(p, _)| p.as_str()).collect();
    let mut edges: BTreeSet<(usize, usize, &'static str)> = BTreeSet::new();
    for (from, (file, source)) in sources.iter().enumerate() {
        for target in targets(file, source, roots) {
            for (to, candidate) in paths.iter().enumerate() {
                let matches = match target.strip_suffix('/') {
                    Some(directory) => {
                        dir(candidate) == directory && lang::of(candidate) == Lang::Go
                    }
                    None => *candidate == target,
                };
                if matches && to != from {
                    edges.insert((from, to, "import"));
                }
            }
        }
        if lang::of(file) == Lang::Script {
            for captures in PATH_LITERAL.captures_iter(source) {
                let key = route_key(&captures[1]);
                for (declared_in, route) in routes {
                    if route_key(route) == key
                        && let Some(to) = paths.iter().position(|p| p == declared_in)
                    {
                        edges.insert((from, to, "http"));
                    }
                }
            }
        }
    }
    let connected: BTreeSet<usize> = edges.iter().flat_map(|(a, b, _)| [*a, *b]).collect();
    let index: Vec<usize> = connected.into_iter().collect();
    let position = |old: usize| index.iter().position(|i| *i == old).unwrap_or_default();
    FileGraph {
        nodes: index.iter().map(|i| paths[*i].to_string()).collect(),
        edges: edges
            .into_iter()
            .map(|(from, to, via)| GraphEdge {
                from: position(from),
                to: position(to),
                via: via.into(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_go_packages_script_imports_and_http_calls() {
        let roots = Roots {
            go_modules: vec![("backend/src".into(), "example".into())],
            packages: vec!["frontend".into()],
            crates: vec![],
        };
        let sources = vec![
            ("backend/src/api/series.go".to_string(), "package api\n\nimport (\n\t\"context\"\n\t\"example/local/usecases\"\n)\n\nfunc init() { r.POST(\"/api/series\", h) }\n".to_string()),
            ("backend/src/local/usecases/create_series.go".to_string(), "package usecases\n".to_string()),
            ("frontend/src/ui/SeriesDialog.tsx".to_string(), "import { create } from '../api/series';\nimport x from '@/lib/date';\n".to_string()),
            ("frontend/src/api/series.ts".to_string(), "export const create = () => fetch(`/api/series`);\n".to_string()),
            ("frontend/src/lib/date.ts".to_string(), String::new()),
            ("README.md".to_string(), String::new()),
        ];
        let routes = vec![(
            "backend/src/api/series.go".to_string(),
            "/api/series".to_string(),
        )];
        let graph = graph(&sources, &roots, &routes);
        let edge = |from: &str, to: &str, via: &str| {
            let f = graph.nodes.iter().position(|n| n == from).unwrap();
            let t = graph.nodes.iter().position(|n| n == to).unwrap();
            graph
                .edges
                .iter()
                .any(|e| e.from == f && e.to == t && e.via == via)
        };
        assert!(edge(
            "backend/src/api/series.go",
            "backend/src/local/usecases/create_series.go",
            "import"
        ));
        assert!(edge(
            "frontend/src/ui/SeriesDialog.tsx",
            "frontend/src/api/series.ts",
            "import"
        ));
        assert!(edge(
            "frontend/src/ui/SeriesDialog.tsx",
            "frontend/src/lib/date.ts",
            "import"
        ));
        assert!(edge(
            "frontend/src/api/series.ts",
            "backend/src/api/series.go",
            "http"
        ));
        assert!(
            !graph.nodes.contains(&"README.md".to_string()),
            "isolated files are omitted"
        );
        assert_eq!(
            route_key("/api/series/${id}"),
            route_key("/api/series/{id}")
        );
    }
}

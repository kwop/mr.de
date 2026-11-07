//! Language detection from file paths.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Go,
    Script,
    Rust,
    Python,
    Sql,
    Other,
}

pub fn of(path: &str) -> Lang {
    let extension = path.rsplit_once('.').map(|(_, e)| e).unwrap_or_default();
    match extension {
        "go" => Lang::Go,
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "vue" | "svelte" => Lang::Script,
        "rs" => Lang::Rust,
        "py" => Lang::Python,
        "sql" => Lang::Sql,
        _ => Lang::Other,
    }
}

/// Path specs covering a language family, for repository searches.
pub fn pathspecs(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Go => &["*.go"],
        Lang::Script => &[
            "*.ts", "*.tsx", "*.js", "*.jsx", "*.mjs", "*.cjs", "*.vue", "*.svelte",
        ],
        Lang::Rust => &["*.rs"],
        Lang::Python => &["*.py"],
        Lang::Sql => &["*.sql"],
        Lang::Other => &[],
    }
}

pub fn is_test(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.ends_with("_test.go")
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.starts_with("test_")
        || name.ends_with("_test.py")
        || name.ends_with("_tests.rs")
        || path.split('/').any(|part| {
            matches!(
                part,
                "test" | "tests" | "__tests__" | "e2e" | "spec" | "testdata"
            )
        })
}

/// Generated or vendored content that readers usually skip.
pub fn is_generated(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "go.sum"
            | "Cargo.lock"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "poetry.lock"
    ) || name.contains(".generated.")
        || name.ends_with(".pb.go")
        || name.ends_with("_gen.go")
        || name.ends_with(".min.js")
        || path.split('/').any(|p| {
            matches!(
                p,
                "vendor" | "node_modules" | "dist" | "generated" | "mocks"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_languages_tests_and_generated_files() {
        assert_eq!(of("a/b.tsx"), Lang::Script);
        assert_eq!(of("migrations/001.up.sql"), Lang::Sql);
        assert_eq!(of("Makefile"), Lang::Other);
        assert!(is_test("src/usecases/create_series_test.go"));
        assert!(is_test("ui/SeriesDialog.test.tsx"));
        assert!(!is_test("src/testing_utils.go"));
        assert!(is_generated("backend/src/go.sum"));
        assert!(is_generated("domain/mocks/repo_mock.go"));
    }
}

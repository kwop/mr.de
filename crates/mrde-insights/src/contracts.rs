//! Changes to contracts other systems rely on: database schema, HTTP endpoints,
//! configuration, permissions, and API schemas.
use crate::diff::FileDiff;
use mrde_core::insights::{ContractCategory, ContractChange, ContractSeverity};
use regex::Regex;
use std::sync::LazyLock;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid contract pattern")
}

static ROUTE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r#"\.(GET|POST|PUT|PATCH|DELETE|Get|Post|Put|Patch|Delete|get|post|put|patch|delete|HandleFunc|Handle|Route|Group|Any|Method)\(\s*(?:"[A-Z]+"\s*,\s*)?["'`](/[^"'`]*)["'`]"#,
    )
});
static OPENAPI_PATH: LazyLock<Regex> =
    LazyLock::new(|| re(r#"^\s{1,4}["']?(/[\w/{}.\-]*)["']?:\s*$"#));
static ENV: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r#"(?:os\.Getenv|os\.LookupEnv|env::var|std::env::var|getenv)\(\s*["']([A-Z][A-Z0-9_]+)["']|process\.env\.([A-Z][A-Z0-9_]+)|import\.meta\.env\.([A-Z][A-Z0-9_]+)|(?:env|envconfig):"([A-Z][A-Z0-9_]+)"|os\.environ(?:\.get)?\(?\[?\s*["']([A-Z][A-Z0-9_]+)["']"#,
    )
});
static PERMISSION: LazyLock<Regex> = LazyLock::new(|| re(r#"["']([a-z][a-z_]*:[a-z][a-z_]*)["']"#));
static CREATE_TABLE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\bcreate\s+table\s+(?:if\s+not\s+exists\s+)?([\w.\x22]+)"));
static DROP: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\bdrop\s+(table|column|index|type|view)\s+(?:if\s+exists\s+)?([\w.\x22]+)")
});
static ADD_COLUMN: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)\balter\s+table\s+([\w.\x22]+)\s+add\s+(?:column\s+)?(?:if\s+not\s+exists\s+)?([\w\x22]+)(.*)",
    )
});
static ALTER_TYPE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)\balter\s+(?:table\s+[\w.\x22]+\s+)?(?:alter\s+)?column\s+([\w\x22]+)\s+(?:set\s+data\s+)?type\b",
    )
});
static RENAME: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\brename\s+(?:column\s+)?([\w\x22]+)\s+to\s+([\w\x22]+)"));
static INDEX: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\bcreate\s+(?:unique\s+)?index\s+(concurrently\s+)?"));
static SET_NOT_NULL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\bset\s+not\s+null\b"));
static SCHEMA_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^\s+(\w+)\s*(?:\([^)]*\))?\s*:\s*[\w!\[\]]+|^\s+(?:optional\s+|repeated\s+|required\s+)?[\w.]+\s+(\w+)\s*=\s*\d+",
    )
});

fn change(
    category: ContractCategory,
    severity: ContractSeverity,
    title: String,
    detail: &str,
    path: &str,
    line: u32,
) -> ContractChange {
    ContractChange {
        category,
        severity,
        title,
        detail: detail.into(),
        path: path.into(),
        line,
    }
}

fn is_migration(path: &str) -> bool {
    path.ends_with(".sql")
        || path.contains("/migrations/")
        || path.contains("/migrate/")
        || path.ends_with("schema.prisma")
}

/// HTTP routes declared on added lines, as (path, route).
pub fn added_routes(files: &[FileDiff]) -> Vec<(String, String)> {
    files
        .iter()
        .flat_map(|file| {
            file.added_lines()
                .filter_map(|(_, line)| {
                    ROUTE
                        .captures(line)
                        .map(|c| (file.path().to_string(), c[2].to_string()))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn database(file: &FileDiff, files: &[FileDiff], out: &mut Vec<ContractChange>) {
    let path = file.path();
    let db = ContractCategory::Database;
    for (line, text) in file.added_lines() {
        if let Some(c) = CREATE_TABLE.captures(text) {
            out.push(change(
                db,
                ContractSeverity::Info,
                format!("New table {}", &c[1]),
                "CREATE TABLE",
                path,
                line,
            ));
        }
        if let Some(c) = DROP.captures(text) {
            out.push(change(
                db,
                ContractSeverity::Breaking,
                format!("DROP {} {}", c[1].to_uppercase(), &c[2]),
                "Data and dependent code are lost; deploy order matters.",
                path,
                line,
            ));
        }
        if let Some(c) = ADD_COLUMN.captures(text) {
            let rest = c[3].to_lowercase();
            if rest.contains("not null") && !rest.contains("default") {
                out.push(change(
                    db,
                    ContractSeverity::Breaking,
                    format!("{}.{} added NOT NULL without default", &c[1], &c[2]),
                    "Fails on tables with existing rows unless backfilled first.",
                    path,
                    line,
                ));
            } else if rest.contains("not null") {
                out.push(change(
                    db,
                    ContractSeverity::Review,
                    format!("{}.{} added NOT NULL with default", &c[1], &c[2]),
                    "May rewrite or lock a large table depending on the database version.",
                    path,
                    line,
                ));
            } else {
                out.push(change(
                    db,
                    ContractSeverity::Info,
                    format!("Column {}.{} added", &c[1], &c[2]),
                    "Nullable column.",
                    path,
                    line,
                ));
            }
        }
        if let Some(c) = ALTER_TYPE.captures(text) {
            out.push(change(
                db,
                ContractSeverity::Review,
                format!("Type of column {} changed", &c[1]),
                "Rewrites the table and takes an exclusive lock.",
                path,
                line,
            ));
        }
        if let Some(c) = RENAME.captures(text) {
            out.push(change(
                db,
                ContractSeverity::Breaking,
                format!("{} renamed to {}", &c[1], &c[2]),
                "Running code still uses the old name during deployment.",
                path,
                line,
            ));
        }
        if SET_NOT_NULL.is_match(text) {
            out.push(change(
                db,
                ContractSeverity::Review,
                "Constraint SET NOT NULL".into(),
                "Scans the whole table under lock.",
                path,
                line,
            ));
        }
        if let Some(c) = INDEX.captures(text)
            && c.get(1).is_none()
        {
            out.push(change(
                db,
                ContractSeverity::Review,
                "Index created without CONCURRENTLY".into(),
                "Blocks writes on the table while the index builds.",
                path,
                line,
            ));
        }
    }
    if file.old_path.is_none() && path.ends_with(".up.sql") {
        let down = path.replace(".up.sql", ".down.sql");
        if !files.iter().any(|f| f.path() == down) {
            out.push(change(
                db,
                ContractSeverity::Review,
                "Migration without a down script".into(),
                "No matching .down.sql in this MR.",
                path,
                1,
            ));
        }
    }
}

/// `env_docs` holds the contents of files documenting configuration (env samples).
pub fn detect(files: &[FileDiff], env_docs: &[String]) -> Vec<ContractChange> {
    let mut out = Vec::new();
    for file in files {
        let path = file.path();
        if file.binary {
            continue;
        }
        if is_migration(path) {
            database(file, files, &mut out);
        }
        let http = ContractCategory::Http;
        let added: Vec<String> = file
            .added_lines()
            .filter_map(|(_, l)| {
                ROUTE
                    .captures(l)
                    .map(|c| format!("{} {}", c[1].to_uppercase(), &c[2]))
            })
            .collect();
        for (line, text) in file.added_lines() {
            if let Some(c) = ROUTE.captures(text) {
                out.push(change(
                    http,
                    ContractSeverity::Info,
                    format!("New endpoint {} {}", c[1].to_uppercase(), &c[2]),
                    "",
                    path,
                    line,
                ));
            }
        }
        for (line, text) in file.removed_lines() {
            if let Some(c) = ROUTE.captures(text) {
                let route = format!("{} {}", c[1].to_uppercase(), &c[2]);
                if !added.contains(&route) {
                    out.push(change(
                        http,
                        ContractSeverity::Breaking,
                        format!("Endpoint removed {route}"),
                        "Clients calling it will fail.",
                        path,
                        line,
                    ));
                }
            }
        }
        let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
        if name.contains("openapi") || name.contains("swagger") {
            for (line, text) in file.removed_lines() {
                if let Some(c) = OPENAPI_PATH.captures(text) {
                    out.push(change(
                        ContractCategory::Schema,
                        ContractSeverity::Breaking,
                        format!("OpenAPI path removed {}", &c[1]),
                        "",
                        path,
                        line,
                    ));
                }
            }
        }
        if path.ends_with(".graphql") || path.ends_with(".gql") || path.ends_with(".proto") {
            let added_fields: Vec<String> = file
                .added_lines()
                .filter_map(|(_, l)| {
                    SCHEMA_FIELD
                        .captures(l)
                        .and_then(|c| c.get(1).or(c.get(2)))
                        .map(|m| m.as_str().to_string())
                })
                .collect();
            for (line, text) in file.removed_lines() {
                if let Some(field) = SCHEMA_FIELD
                    .captures(text)
                    .and_then(|c| c.get(1).or(c.get(2)))
                    && !added_fields.iter().any(|f| f == field.as_str())
                {
                    out.push(change(
                        ContractCategory::Schema,
                        ContractSeverity::Breaking,
                        format!("Schema field removed: {}", field.as_str()),
                        "Existing clients may still read it.",
                        path,
                        line,
                    ));
                }
            }
        }
        let config = ContractCategory::Configuration;
        for (line, text) in file.added_lines() {
            for c in ENV.captures_iter(text) {
                let Some(var) = (1..=5).find_map(|i| c.get(i)).map(|m| m.as_str()) else {
                    continue;
                };
                if out
                    .iter()
                    .any(|o: &ContractChange| o.category == config && o.title.ends_with(var))
                {
                    continue;
                }
                let documented = env_docs.iter().any(|doc| {
                    doc.lines().any(|l| {
                        l.trim_start()
                            .trim_start_matches("export ")
                            .starts_with(&format!("{var}="))
                            || l.contains(&format!("{var}:"))
                    })
                });
                if documented {
                    out.push(change(
                        config,
                        ContractSeverity::Info,
                        format!("Uses variable {var}"),
                        "Documented in an environment sample.",
                        path,
                        line,
                    ));
                } else {
                    out.push(change(config, ContractSeverity::Review, format!("New variable {var}"), "Not found in any environment sample (env-sample, .env.example, compose files).", path, line));
                }
            }
        }
        let lower = path.to_lowercase();
        if ["permission", "rbac", "policy", "policies", "acl", "role"]
            .iter()
            .any(|k| lower.contains(k))
        {
            for (line, text) in file.added_lines() {
                for c in PERMISSION.captures_iter(text) {
                    out.push(change(
                        ContractCategory::Permissions,
                        ContractSeverity::Review,
                        format!("Permission {}", &c[1]),
                        "New or changed authorization rule.",
                        path,
                        line,
                    ));
                }
            }
        }
    }
    out.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
    });
    out
}

/// Files that document configuration: env samples and compose/helm values.
pub fn is_env_doc(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    (name.contains("env")
        && (name.contains("sample")
            || name.contains("example")
            || name.contains("template")
            || name.ends_with(".env")))
        || name.starts_with("docker-compose")
        || name.starts_with("compose.")
        || name == "values.yaml"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff;

    #[test]
    fn detects_schema_routes_configuration_and_permissions() {
        let raw = "diff --git a/m/000041.up.sql b/m/000041.up.sql\nnew file mode 100644\n--- /dev/null\n+++ b/m/000041.up.sql\n@@ -0,0 +1,4 @@\n+CREATE TABLE invoice_schedules (id uuid);\n+ALTER TABLE invoices ADD COLUMN schedule_id uuid NOT NULL;\n+CREATE INDEX idx_schedule ON invoices (schedule_id);\n+ALTER TABLE invoices ADD COLUMN note text;\n\
diff --git a/api/routes.go b/api/routes.go\n--- a/api/routes.go\n+++ b/api/routes.go\n@@ -3 +3,2 @@\n-\tr.POST(\"/api/invoices/recurrence\", h)\n+\tr.POST(\"/api/schedules\", h.Create)\n+\tlimit := os.Getenv(\"SCHEDULE_MAX_OCCURRENCES\")\n\
diff --git a/auth/permissions.go b/auth/permissions.go\n--- a/auth/permissions.go\n+++ b/auth/permissions.go\n@@ -1,0 +2 @@\n+\tGrant(\"schedules:create\", Manager)\n";
        let files = diff::parse(raw);
        let found = detect(&files, &["DATABASE_URL=postgres://\n".into()]);
        let titles: Vec<(&str, ContractSeverity)> = found
            .iter()
            .map(|c| (c.title.as_str(), c.severity))
            .collect();
        assert!(titles.contains(&(
            "invoices.schedule_id added NOT NULL without default",
            ContractSeverity::Breaking
        )));
        assert!(titles.contains(&(
            "Endpoint removed POST /api/invoices/recurrence",
            ContractSeverity::Breaking
        )));
        assert!(titles.contains(&("New table invoice_schedules", ContractSeverity::Info)));
        assert!(titles.contains(&(
            "Index created without CONCURRENTLY",
            ContractSeverity::Review
        )));
        assert!(titles.contains(&("Migration without a down script", ContractSeverity::Review)));
        assert!(titles.contains(&("New endpoint POST /api/schedules", ContractSeverity::Info)));
        assert!(titles.contains(&(
            "New variable SCHEDULE_MAX_OCCURRENCES",
            ContractSeverity::Review
        )));
        assert!(titles.contains(&("Permission schedules:create", ContractSeverity::Review)));
        assert_eq!(
            found[0].severity,
            ContractSeverity::Breaking,
            "breaking first"
        );
        assert_eq!(
            added_routes(&files),
            [("api/routes.go".to_string(), "/api/schedules".to_string())]
        );
        assert!(
            is_env_doc("backend/env-sample")
                && is_env_doc(".env.example")
                && !is_env_doc("src/env.go")
        );
    }
}

//! Synthetic data for `--ui-preview`: realistic merge requests for visual inspection and
//! screenshots, without GitLab, Git, Claude, or Keychain access.
use mrde_core::models::*;

pub const INSTANCE: &str = "https://gitlab.example.com";

fn label(name: &str, color: &str) -> Label {
    Label {
        name: name.into(),
        color: Some(color.into()),
        text_color: None,
    }
}

fn sha(seed: u64) -> String {
    let mut state = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    (0..5)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            format!("{:08x}", state as u32)
        })
        .collect()
}

struct Spec {
    iid: u64,
    title: &'static str,
    author: &'static str,
    hours: i64,
    pipeline: Option<&'static str>,
    labels: &'static [(&'static str, &'static str)],
    lines: Option<u64>,
    approvals: Option<u64>,
}

const SPECS: &[Spec] = &[
    Spec {
        iid: 231,
        title: "US-3.4 — Create recurring invoices from the invoice form, fix platform#1203",
        author: "alice",
        hours: 30,
        pipeline: Some("success"),
        labels: &[
            ("P::1", "#e2a543"),
            ("Type::Feature", "#428bca"),
            ("billing", "#6c757d"),
        ],
        lines: Some(1730),
        approvals: Some(1),
    },
    Spec {
        iid: 238,
        title: "US-3.10 — Cancel an invoice with a reason and notify the customer",
        author: "bob",
        hours: 8,
        pipeline: Some("failed"),
        labels: &[
            ("Review::Dev", "#6699cc"),
            ("P::0", "#dc143c"),
            ("Type::Feature", "#428bca"),
        ],
        lines: Some(412),
        approvals: Some(0),
    },
    Spec {
        iid: 229,
        title: "US-3.11 — Payer and billing contact on invoices",
        author: "carol",
        hours: 52,
        pipeline: Some("running"),
        labels: &[("P::2", "#5cb85c"), ("accounting", "#8e44ad")],
        lines: Some(268),
        approvals: Some(2),
    },
    Spec {
        iid: 244,
        title: "Refactor: extract the notifications module (email, SMS, webhooks, delivery queue)",
        author: "dave",
        hours: 120,
        pipeline: Some("success"),
        labels: &[
            ("Type::Refactor", "#69d100"),
            ("backend", "#330066"),
            ("needs-review", "#ffecdb"),
        ],
        lines: Some(6420),
        approvals: None,
    },
    Spec {
        iid: 245,
        title: "fix: invoice dated the day before in UTC+1 (#1187)",
        author: "erin",
        hours: 3,
        pipeline: Some("success"),
        labels: &[("P::0", "#dc143c"), ("Type::Bug", "#d9534f")],
        lines: Some(18),
        approvals: Some(0),
    },
    Spec {
        iid: 241,
        title: "Support chat: attachments and read receipts",
        author: "carol",
        hours: 70,
        pipeline: Some("manual"),
        labels: &[("frontend", "#0033cc")],
        lines: Some(940),
        approvals: Some(1),
    },
    Spec {
        iid: 236,
        title: "Calendar: drag and drop events between two weeks",
        author: "dave",
        hours: 200,
        pipeline: None,
        labels: &[("P::3", "#8fbc8f"), ("frontend", "#0033cc")],
        lines: None,
        approvals: None,
    },
    Spec {
        iid: 233,
        title: "Accounting export with the new VAT rules",
        author: "bob",
        hours: 340,
        pipeline: Some("success"),
        labels: &[("accounting", "#8e44ad"), ("tax", "#ad4363")],
        lines: Some(2210),
        approvals: Some(0),
    },
];

pub fn queue() -> Vec<MergeRequest> {
    let now = chrono::Utc::now();
    let mut queue: Vec<MergeRequest> = SPECS
        .iter()
        .map(|spec| MergeRequest {
            id: MrId {
                project: 42,
                iid: spec.iid,
            },
            title: spec.title.into(),
            description: String::new(),
            project: "acme/platform".into(),
            author: spec.author.into(),
            web_url: format!("{INSTANCE}/acme/platform/-/merge_requests/{}", spec.iid),
            sha: sha(spec.iid),
            base_sha: sha(spec.iid + 1_000),
            created_at: now - chrono::Duration::hours(spec.hours),
            // As on many teams: review state on the MR, the rest on its ticket.
            labels: spec
                .labels
                .iter()
                .filter(|(n, _)| n.starts_with("Review::"))
                .map(|(n, c)| label(n, c))
                .collect(),
            ticket_labels: spec
                .labels
                .iter()
                .filter(|(n, _)| !n.starts_with("Review::"))
                .map(|(n, c)| label(n, c))
                .collect(),
            pipeline: spec.pipeline.map(str::to_string),
            approvals: spec.approvals,
            changed_lines: spec.lines,
        })
        .collect();
    // Same order as a real refresh: priority labels, readiness, age, size.
    mrde_core::scoring::sort_queue(&mut queue, mrde_core::config::QueueSort::Priority, now);
    queue
}

const DESCRIPTION: &str = r#"Closes #1203 · Spec: `specs/billing/us-3.4-recurring-invoices`

## Context

Account managers create every invoice of a subscription **by hand** (monthly, quarterly, or yearly plans). This MR generates a schedule of invoices from a recurrence rule.

## Changes

- New `InvoiceSchedule` entity and `000041_invoice_schedules` migration.
- `CreateSchedule` use case: computes the occurrences (weekly or monthly, chosen days, end after a count or on a date).
- `POST /api/schedules` and `GET /api/schedules/{id}` endpoints.
- "Create a schedule" dialog and "Bill only the first period" toggle.

```mermaid
flowchart LR
  F[Invoice form] --> D[Schedule dialog]
  D -->|rule| API[POST /api/schedules]
  API --> UC[CreateSchedule]
  UC --> DB[(invoices)]
```

| Frequency | End of schedule | Maximum occurrences |
|---|---|---|
| Monthly | Count | 24 |
| Weekly, chosen days (Monday, Wednesday, Friday) | End date entered in the dialog, validated by the API before any invoice is created | 52; longer schedules are refused with a message in the dialog |

![Schedule dialog](/uploads/0123456789abcdef0123456789abcdef/schedule.png)

> Schedules longer than 52 occurrences are refused for now.
"#;

const FILES: &[(&str, u64, u64)] = &[
    ("backend/internal/domain/schedule.go", 142, 0),
    ("backend/internal/domain/invoice.go", 6, 2),
    ("backend/internal/usecases/create_schedule.go", 188, 0),
    ("backend/internal/usecases/create_invoice.go", 22, 41),
    ("backend/internal/usecases/validate_schedule.go", 12, 9),
    ("backend/internal/repositories/schedule_repo.go", 96, 0),
    ("backend/internal/repositories/invoice_repo.go", 14, 3),
    ("backend/api/handlers/schedules.go", 120, 0),
    ("backend/api/handlers/invoices.go", 18, 12),
    ("backend/api/routes.go", 4, 0),
    ("backend/migrations/000041_invoice_schedules.up.sql", 32, 0),
    ("backend/migrations/000041_invoice_schedules.down.sql", 6, 0),
    ("backend/go.mod", 2, 1),
    ("backend/go.sum", 4, 2),
    ("backend/internal/usecases/create_schedule_test.go", 210, 0),
    ("backend/internal/domain/schedule_test.go", 96, 0),
    ("backend/internal/mocks/schedule_repo_mock.go", 64, 0),
    ("web/src/features/invoices/ui/ScheduleDialog.tsx", 190, 0),
    ("web/src/features/invoices/ui/InvoiceForm.tsx", 48, 31),
    ("web/src/features/invoices/ui/RecurrenceFields.tsx", 84, 0),
    ("web/src/features/invoices/api/schedules.ts", 38, 0),
    ("web/src/features/invoices/lib/useCreateInvoice.ts", 22, 9),
    ("web/src/features/invoices/index.ts", 2, 0),
    (
        "web/src/features/invoices/ui/ScheduleDialog.test.tsx",
        88,
        0,
    ),
    ("web/package.json", 2, 1),
    ("web/pnpm-lock.yaml", 40, 12),
    (
        "specs/billing/us-3.4-recurring-invoices/revisions/rev-8.md",
        14,
        0,
    ),
];

fn diff(added: u64, removed: u64) -> String {
    let mut text = format!("@@ -1,{removed} +1,{added} @@\n");
    for i in 0..removed {
        text.push_str(&format!("-old line {i}\n"));
    }
    for i in 0..added {
        text.push_str(&format!("+new line {i}\n"));
    }
    text
}

pub fn detail(id: &MrId) -> Option<MrDetail> {
    let mut mr = queue().into_iter().find(|mr| &mr.id == id)?;
    let files: Vec<ChangedFile> = if mr.id.iid == 244 {
        (0..180)
            .map(|i| {
                let path = format!(
                    "backend/internal/notifications/{}/{}_{i}.go",
                    ["email", "sms", "webhooks", "queue", "templates"][i % 5],
                    ["sender", "payload", "consumer", "handler"][i % 4]
                );
                ChangedFile {
                    old_path: path.clone(),
                    new_path: path,
                    diff: diff(10 + (i as u64 * 7) % 90, (i as u64 * 3) % 40),
                    too_large: false,
                    collapsed: false,
                }
            })
            .collect()
    } else {
        FILES
            .iter()
            .map(|(path, added, removed)| ChangedFile {
                old_path: path.to_string(),
                new_path: path.to_string(),
                diff: diff(*added, *removed),
                too_large: false,
                collapsed: false,
            })
            .collect()
    };
    mr.description = if mr.id.iid == 231 {
        DESCRIPTION.into()
    } else {
        format!(
            "## Summary\n\n{}.\n\n- First change\n- Second change\n",
            mr.title
        )
    };
    let ticket_labels = mr.ticket_labels.clone();
    Some(MrDetail {
        mr,
        files,
        issues: vec![Issue {
            title: "#1203 Create recurring invoices".into(),
            description: "As an account manager, I want to create a schedule of recurring invoices so that I do not enter each invoice by hand.\n\n**Criteria**: weekly or monthly, chosen days, end after a count or on a date.".into(),
            // Recent GitLab versions link tickets as work items.
            web_url: format!("{INSTANCE}/acme/platform/-/work_items/1203"),
            labels: ticket_labels,
            project_id: Some(42),
        }],
        ssh_url: "git@gitlab.example.com:acme/platform.git".into(),
    })
}

pub fn review() -> Review {
    serde_json::from_value(serde_json::json!({
        "summary": "The MR adds recurring invoice schedules, from the migration to the dialog. Occurrences are computed correctly for weekly and monthly rules; the \"due date before issue date\" validation was removed for schedules.",
        "intention": "Let account managers generate many recurring invoices without entering each one.",
        "risks": [
            {"severity": "high", "description": "The \"due date before issue date\" guard (incident #1187) is removed from `create_invoice.go` with no equivalent for schedules.", "file": "backend/internal/usecases/create_invoice.go"},
            {"severity": "medium", "description": "Occurrences are inserted one by one in a loop: up to 52 queries per schedule.", "file": "backend/internal/usecases/create_schedule.go"},
            {"severity": "low", "description": "The \"Add schedule\" button uses the primary variant, contrary to revision 8 of the spec.", "file": "web/src/features/invoices/ui/ScheduleDialog.tsx"}
        ],
        "priority_files": [
            "backend/internal/usecases/create_schedule.go",
            "backend/internal/usecases/create_invoice.go",
            "backend/migrations/000041_invoice_schedules.up.sql"
        ],
        "confidence": 0.78
    }))
    .expect("valid demo review")
}

pub fn compliance() -> Compliance {
    const SPEC: &str = "billing/us-3.4-recurring-invoices/spec.gherkin";
    const REVISION: &str = "billing/us-3.4-recurring-invoices/revisions/rev-8.md";
    serde_json::from_value(serde_json::json!({
        "verdict": "partial",
        "summary": "Weekly and monthly schedules and both end conditions comply. Two gaps: the \"Add schedule\" button is primary (revision 8), and the 52-occurrence limit is not shown to the user.",
        "spec_files": [SPEC, REVISION],
        "findings": [
            {"requirement": "Choose the \"Recurring\" mode", "status": "met", "evidence": "InvoiceForm.tsx shows the schedule options.", "spec_file": SPEC, "code": ["web/src/features/invoices/ui/InvoiceForm.tsx:44"], "tests": ["web/e2e/schedules.spec.ts"]},
            {"requirement": "\"Bill only the first period\" mode", "status": "met", "evidence": "The toggle exists; only the first invoice date is shown.", "spec_file": SPEC, "code": ["web/src/features/invoices/ui/InvoiceForm.tsx:61"], "tests": []},
            {"requirement": "\"Add schedule\" is an outlined button", "status": "missing", "evidence": "ScheduleDialog.tsx:30 uses variant=\"primary\".", "spec_file": REVISION, "code": ["web/src/features/invoices/ui/ScheduleDialog.tsx:30"], "tests": []},
            {"requirement": "Weekly frequency with chosen days", "status": "met", "evidence": "schedule.go computes one occurrence per enabled day.", "spec_file": SPEC, "code": ["backend/internal/domain/schedule.go:52"], "tests": ["backend/internal/domain/schedule_test.go"]},
            {"requirement": "Schedules longer than 52 occurrences", "status": "partial", "evidence": "Refused by the API, with no message in the dialog.", "spec_file": SPEC, "code": ["backend/api/handlers/schedules.go:40"], "tests": []}
        ],
        "confidence": 0.71
    }))
    .expect("valid demo compliance")
}

pub fn stats(detail: &MrDetail) -> DiffStats {
    DiffStats {
        sha: detail.mr.sha.clone(),
        source: StatsSource::Git,
        files: detail
            .files
            .iter()
            .map(mrde_core::stats::file_stat)
            .collect(),
    }
}

pub fn worktrees() -> Vec<mrde_git::WorktreeEntry> {
    [(231, Some(0)), (238, Some(3)), (244, Some(0))]
        .into_iter()
        .map(|(iid, changes)| mrde_git::WorktreeEntry {
            project: 42,
            iid,
            sha: sha(iid),
            path: format!("~/Library/Caches/mrde/git/demo/42-{iid}-{}", sha(iid)).into(),
            changes,
        })
        .collect()
}

/// A small screenshot-like image for Markdown image previews.
pub fn image() -> Vec<u8> {
    r##"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="300"><rect width="640" height="300" rx="12" fill="#26262a"/><rect x="24" y="24" width="592" height="40" rx="6" fill="#33333a"/><text x="44" y="50" font-family="Helvetica" font-size="16" fill="#e6e6e6">Create a schedule</text><rect x="24" y="84" width="280" height="36" rx="6" fill="#3f6ea6"/><text x="44" y="108" font-family="Helvetica" font-size="14" fill="#fff">Monthly · 1st and 15th</text><rect x="24" y="236" width="180" height="40" rx="8" fill="#4dc179"/><text x="44" y="262" font-family="Helvetica" font-size="15" fill="#10241a">Add schedule</text></svg>"##
        .as_bytes()
        .to_vec()
}

#[allow(clippy::too_many_arguments)]
fn symbol(
    path: &str,
    name: &str,
    kind: &str,
    change: &str,
    before: Option<&str>,
    after: Option<&str>,
    added: u32,
    removed: u32,
    line: u32,
) -> serde_json::Value {
    serde_json::json!({"path": path, "name": name, "kind": kind, "change": change, "before": before, "after": after, "added": added, "removed": removed, "line": line})
}

pub fn insights(detail: &MrDetail) -> mrde_core::insights::Insights {
    const B: &str = "backend/internal/";
    const W: &str = "web/src/features/invoices/";
    let value = serde_json::json!({
        "sha": detail.mr.sha,
        "stats": stats(detail),
        "symbols": [
            symbol(&format!("{B}usecases/validate_schedule.go"), "ValidateSchedule", "function", "signature_changed", Some("func ValidateSchedule(i Invoice) error {"), Some("func ValidateSchedule(i Invoice, recurring bool) error {"), 12, 9, 14),
            symbol(&format!("{B}usecases/create_invoice.go"), "duplicateInvoice", "function", "removed", Some("func duplicateInvoice(i Invoice) Invoice {"), None, 0, 41, 120),
            symbol(&format!("{B}domain/invoice.go"), "Invoice", "type", "modified", Some("type Invoice struct {"), Some("type Invoice struct {"), 3, 0, 18),
            symbol(&format!("{W}lib/useCreateInvoice.ts"), "useCreateInvoice", "function", "modified", Some("export function useCreateInvoice() {"), Some("export function useCreateInvoice() {"), 22, 9, 8),
            symbol(&format!("{B}domain/schedule.go"), "InvoiceSchedule", "type", "added", None, Some("type InvoiceSchedule struct {"), 38, 0, 9),
            symbol(&format!("{B}domain/schedule.go"), "InvoiceSchedule.Occurrences", "method", "added", None, Some("func (s InvoiceSchedule) Occurrences() []time.Time {"), 52, 0, 48),
            symbol(&format!("{B}usecases/create_schedule.go"), "CreateSchedule", "function", "added", None, Some("func CreateSchedule(ctx context.Context, cmd CreateScheduleCommand) (Schedule, error) {"), 88, 0, 21),
            symbol(&format!("{W}ui/ScheduleDialog.tsx"), "ScheduleDialog", "function", "added", None, Some("export function ScheduleDialog({ onClose }: Props) {"), 190, 0, 12)
        ],
        "impacts": [{
            "symbol": "ValidateSchedule", "path": format!("{B}usecases/validate_schedule.go"), "change": "signature_changed", "more": 0,
            "callers": [
                {"path": "backend/cmd/import/import_csv.go", "line": 142, "text": "if err := usecases.ValidateSchedule(i); err != nil {", "in_mr": false, "test": false},
                {"path": "backend/internal/services/reminder_bot.go", "line": 77, "text": "return usecases.ValidateSchedule(next)", "in_mr": false, "test": false},
                {"path": "backend/internal/webhooks/payment.go", "line": 203, "text": "_ = usecases.ValidateSchedule(invoice)", "in_mr": false, "test": false},
                {"path": format!("{B}usecases/create_schedule.go"), "line": 61, "text": "if err := ValidateSchedule(i, true); err != nil {", "in_mr": true, "test": false},
                {"path": format!("{B}usecases/create_invoice.go"), "line": 84, "text": "if err := ValidateSchedule(i, false); err != nil {", "in_mr": true, "test": false},
                {"path": format!("{B}usecases/create_schedule_test.go"), "line": 33, "text": "assert.NoError(t, ValidateSchedule(inv, true))", "in_mr": true, "test": true}
            ]
        }],
        "graph": {
            "nodes": [
                format!("{W}ui/InvoiceForm.tsx"), format!("{W}ui/ScheduleDialog.tsx"), format!("{W}api/schedules.ts"),
                "backend/api/handlers/schedules.go", format!("{B}usecases/create_schedule.go"),
                format!("{B}domain/schedule.go"), format!("{B}repositories/schedule_repo.go"),
                format!("{B}usecases/validate_schedule.go")
            ],
            "edges": [
                {"from": 0, "to": 1, "via": "import"}, {"from": 1, "to": 2, "via": "import"}, {"from": 2, "to": 3, "via": "http"},
                {"from": 3, "to": 4, "via": "import"}, {"from": 4, "to": 5, "via": "import"}, {"from": 4, "to": 6, "via": "import"},
                {"from": 4, "to": 7, "via": "import"}
            ]
        },
        "removed": [{
            "path": format!("{B}usecases/create_invoice.go"), "line": 84, "pure_deletion": true,
            "lines": ["if cmd.DueAt != nil && cmd.DueAt.Before(cmd.IssuedAt) {", "    return Invoice{}, apierrors.InvalidSchedule", "}"],
            "origin": {"sha": "9f3c2e1a5b7d", "author": "Erin M.", "date": "2026-03-12", "summary": "fix: reject a due date before the issue date (incident #1187)", "references": ["#1187", "!164"]}
        }],
        "history": [
            {"path": format!("{B}usecases/create_invoice.go"), "commits": 47, "fixes": 9, "authors": 6, "last_fix": "2026-03-12 · fix: reject a due date before the issue date (incident #1187)", "risk": "high", "new_file": false},
            {"path": format!("{W}ui/InvoiceForm.tsx"), "commits": 18, "fixes": 2, "authors": 4, "last_fix": "2026-09-30 · fix: empty payer on save", "risk": "medium", "new_file": false},
            {"path": format!("{B}repositories/invoice_repo.go"), "commits": 9, "fixes": 1, "authors": 3, "last_fix": "2026-05-02 · fix: missing index", "risk": "medium", "new_file": false},
            {"path": format!("{B}domain/schedule.go"), "commits": 0, "fixes": 0, "authors": 0, "last_fix": null, "risk": "low", "new_file": true}
        ],
        "experts": [
            {"zone": "backend/internal/usecases", "people": [["Erin M.", 61], ["Bob K.", 22], ["Dave T.", 9]], "owners": ["@erin"]},
            {"zone": "backend/migrations", "people": [["Bob K.", 80], ["Erin M.", 20]], "owners": ["@bob"]},
            {"zone": "web", "people": [["Dave T.", 48], ["Carol R.", 35]], "owners": []}
        ],
        "contracts": [
            {"category": "http", "severity": "breaking", "title": "Endpoint removed POST /api/invoices/recurrence", "detail": "Still called by the mobile app (2 calls).", "path": "backend/api/routes.go", "line": 41},
            {"category": "database", "severity": "review", "title": "invoices.schedule_id added NOT NULL with default", "detail": "May rewrite or lock a large table depending on the database version.", "path": "backend/migrations/000041_invoice_schedules.up.sql", "line": 12},
            {"category": "configuration", "severity": "review", "title": "New variable SCHEDULE_MAX_OCCURRENCES", "detail": "Not found in any environment sample (env-sample, .env.example, compose files).", "path": format!("{B}usecases/create_schedule.go"), "line": 18},
            {"category": "database", "severity": "info", "title": "New table invoice_schedules", "detail": "CREATE TABLE", "path": "backend/migrations/000041_invoice_schedules.up.sql", "line": 1},
            {"category": "http", "severity": "info", "title": "New endpoint POST /api/schedules", "detail": "", "path": "backend/api/routes.go", "line": 52}
        ],
        "dependencies": [
            {"manifest": "backend/go.mod", "name": "github.com/teambition/rrule-go", "before": null, "after": "v1.8.2", "kind": "added", "major": false, "dev": false},
            {"manifest": "backend/go.mod", "name": "github.com/jackc/pgx/v5", "before": "v5.5.4", "after": "v5.7.1", "kind": "upgraded", "major": false, "dev": false},
            {"manifest": "web/package.json", "name": "date-fns", "before": "^2.30.0", "after": "^4.1.0", "kind": "upgraded", "major": true, "dev": false},
            {"manifest": "web/package.json", "name": "rrule", "before": null, "after": "2.8.1", "kind": "added", "major": false, "dev": false}
        ],
        "lockfiles": [{"path": "web/pnpm-lock.yaml", "added": 3, "removed": 0}, {"path": "backend/go.sum", "added": 2, "removed": 1}],
        "patterns": [
            {"level": "high", "rule": "query-in-loop", "title": "Database or network call inside a loop", "path": format!("{B}usecases/create_schedule.go"), "line": 72, "snippet": "repo.Insert(ctx, invoice)", "advice": "Batch the operation (bulk insert, IN query, Promise.all) to avoid N round trips."},
            {"level": "medium", "rule": "ignored-error", "title": "Error ignored", "path": "backend/api/handlers/schedules.go", "line": 55, "snippet": "_ = notifier.Publish(ctx, event)", "advice": "Handle or log the error; failures here are silent."},
            {"level": "medium", "rule": "effect-without-deps", "title": "useEffect without a dependency array", "path": format!("{W}ui/ScheduleDialog.tsx"), "line": 118, "snippet": "useEffect(() => {", "advice": "The effect runs after every render; pass the values it depends on."},
            {"level": "low", "rule": "todo", "title": "TODO left in the change", "path": format!("{W}ui/RecurrenceFields.tsx"), "line": 40, "snippet": "// TODO: yearly", "advice": "Track it in an issue or resolve it before merging."}
        ],
        "overlaps": [
            {"iid": 238, "title": "US-3.10 — Cancel an invoice with a reason", "merged": false, "web_url": format!("{INSTANCE}/acme/platform/-/merge_requests/238"), "files": [format!("{B}usecases/create_invoice.go"), format!("{B}usecases/validate_schedule.go")], "updated_at": "2026-11-06T09:12:00Z"},
            {"iid": 229, "title": "US-3.11 — Payer and billing contact on invoices", "merged": true, "web_url": format!("{INSTANCE}/acme/platform/-/merge_requests/229"), "files": [format!("{B}domain/invoice.go")], "updated_at": "2026-11-04T16:40:00Z"}
        ],
        "warnings": []
    });
    serde_json::from_value(value).expect("valid demo insights")
}

pub fn tour() -> mrde_core::insights::Tour {
    serde_json::from_value(serde_json::json!({
        "minutes": 25,
        "steps": [
            {"title": "Domain model", "summary": "Introduces InvoiceSchedule (recurrence rule, end of schedule) and links each Invoice to its schedule.", "files": ["backend/internal/domain/schedule.go", "backend/internal/domain/invoice.go"], "core": false},
            {"title": "Migration", "summary": "Creates invoice_schedules and adds invoices.schedule_id.", "files": ["backend/migrations/000041_invoice_schedules.up.sql", "backend/migrations/000041_invoice_schedules.down.sql"], "core": false},
            {"title": "Occurrence generation", "summary": "CreateSchedule computes the dates (weekly or monthly, chosen days, end after a count or on a date) and creates the invoices in one transaction. ValidateSchedule gains a recurring flag.", "files": ["backend/internal/usecases/create_schedule.go", "backend/internal/usecases/validate_schedule.go", "backend/internal/usecases/create_invoice.go", "backend/internal/repositories/schedule_repo.go"], "core": true},
            {"title": "API", "summary": "POST /api/schedules and GET /api/schedules/{id}, with recurrence validation.", "files": ["backend/api/handlers/schedules.go", "backend/api/routes.go"], "core": false},
            {"title": "Interface", "summary": "The \"Create a schedule\" dialog and the first-period-only toggle.", "files": ["web/src/features/invoices/ui/ScheduleDialog.tsx", "web/src/features/invoices/ui/InvoiceForm.tsx", "web/src/features/invoices/ui/RecurrenceFields.tsx", "web/src/features/invoices/api/schedules.ts"], "core": false}
        ],
        "outside": ["backend/go.sum", "web/pnpm-lock.yaml", "backend/internal/mocks/schedule_repo_mock.go"],
        "diagram": "sequenceDiagram\n  participant UI as ScheduleDialog\n  participant API as POST /api/schedules NEW\n  participant UC as CreateSchedule NEW\n  participant DB as Postgres\n  UI->>API: monthly rule, 1st and 15th, 12 occurrences\n  API->>UC: Validate(rule)\n  UC->>UC: Occurrences() NEW\n  UC->>DB: INSERT invoice_schedules NEW\n  loop each occurrence\n    UC->>DB: INSERT invoices (schedule_id)\n  end\n  API-->>UI: 201, 12 invoices"
    }))
    .expect("valid demo tour")
}

/// Synthetic before/after screenshots, compared with the real pixel diff.
pub fn visual() -> anyhow::Result<mrde_core::insights::VisualReport> {
    let root = std::env::temp_dir().join("mrde-demo-visual");
    let (before, after, diff) = (root.join("before"), root.join("after"), root.join("diff"));
    let _ = std::fs::remove_dir_all(&root);
    let draw =
        |path: std::path::PathBuf, recurring: bool, green_button: bool| -> anyhow::Result<()> {
            let (width, height) = (640u32, 400u32);
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for y in 0..height {
                for x in 0..width {
                    let inside =
                        |x0: u32, y0: u32, x1: u32, y1: u32| x >= x0 && x < x1 && y >= y0 && y < y1;
                    let color: [u8; 4] = if inside(24, 24, 616, 64) {
                        [52, 56, 64, 255]
                    } else if recurring && inside(24, 96, 300, 132) {
                        [63, 110, 166, 255]
                    } else if inside(24, 160, 300, 196) || inside(320, 160, 616, 196) {
                        [48, 52, 59, 255]
                    } else if inside(24, 330, 220, 372) {
                        if green_button {
                            [77, 193, 121, 255]
                        } else {
                            [48, 52, 59, 255]
                        }
                    } else if recurring && inside(240, 330, 440, 372) {
                        [77, 193, 121, 255]
                    } else {
                        [38, 41, 47, 255]
                    };
                    rgba.extend_from_slice(&color);
                }
            }
            mrde_insights::visual::write_rgba(&path, width, height, rgba)
        };
    draw(before.join("invoice-form.png"), false, true)?;
    draw(after.join("invoice-form.png"), true, false)?;
    draw(before.join("calendar.png"), false, false)?;
    draw(after.join("calendar.png"), false, false)?;
    draw(after.join("schedule-dialog.png"), true, true)?;
    mrde_insights::visual::compare(&before, &after, &diff)
}

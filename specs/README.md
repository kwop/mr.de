# MR.DE product specifications

This directory is the product contract for MR.DE. English Gherkin describes observable
behavior; the native [UI reference](ui/README.md) describes presentation and interaction.
A **project** is an MR.DE connection profile. A **repository** belongs to a forge. Unless a
scenario names a provider, **request** means a GitLab merge request or GitHub pull request.

## Contents

| Feature | Contract |
| --- | --- |
| [Languages](features/languages.feature) | Independent global interface and information languages |
| [Terminal](features/terminal.feature) | Shared-service TUI, interaction, background work and exports |
| [Projects](features/projects.feature) | Profiles, TOML, migration and isolation |
| [Connection settings](features/connection_settings.feature) | Test, Save, secret retention and errors |
| [Inbox](features/inbox.feature) | Eligibility, priority, filters, labels, pagination and refresh |
| [Reader](features/reader.feature) | Navigation, Markdown, files and scrolling |
| [Images](features/images.feature) | Fit, zoom, scrolling, authenticated uploads and failure |
| [Review and chat](features/review_and_chat.feature) | Explicit Claude actions, caching and Threads |
| [Approval](features/approval.feature) | Confirmation and reviewed-commit protection |
| [Analysis](features/analysis.feature) | Structure, statistics, tours, compliance and screenshots |
| [Risk radar](features/risk_radar.feature) | Measurements, n/a, medians, evidence and command approval |
| [Radar CLI and plugin](features/radar_cli.feature) | Headless scans, snapshots, JSON, checks and explicit Claude invocation |
| [Super Resolver](features/super_resolver.feature) | Bounded repair, handover and interruption |
| [Worktrees](features/worktrees.feature) | Reuse and cleanup without losing local work |
| [UI contract](features/ui_contract.feature) | Layout, status, responsiveness and preview isolation |

## Writing and changing a contract

Use two-space indentation and English `Feature`, `Scenario`, `Given`, `When`, `Then`.
Describe a meaningful action and an observable result; keep selectors, Rust types, HTTP
fixtures and process setup in future step definitions. Use Scenario Outlines for equivalent
cases. Keep examples synthetic, including domains, names and credentials.

Each scenario has one stable ID such as `@MRDE-IMAGE-002`. Keep it when wording changes;
allocate a new ID for a different behavior and never recycle a removed ID. `@regression`
marks a previously reported failure, `@smoke` a core workflow, and `@visual` a presentation
contract. `@contract` means expected behavior, **not a passing or automated test**. Mark new
unimplemented expectations `@planned` and explain the gap; do not silently redefine a bug
as the specification.

A feature change should update its contract, relevant Rust regression test and linked UI
story together. Current button names are examples of access points, not an obligation to
keep navigation wording forever. In particular, change-analysis navigation is evolving.

## Validation and future automation

The development tools require Python 3.11 or later.

```sh
python3 -m venv /tmp/mrde-specs-venv
/tmp/mrde-specs-venv/bin/python -m pip install -r specs/requirements.txt
/tmp/mrde-specs-venv/bin/python scripts/check_specs.py
python3 scripts/ui_catalog.py check
python3 -m unittest discover -s scripts/tests
```

The official Gherkin parser checks syntax, examples, stable IDs and story references.
**No application steps are executed.** [Existing Rust tests](coverage.md) cover parts of these
contracts independently. Future [Cucumber Rust](https://cucumber-rs.github.io/cucumber/current/)
bindings can reuse domain services, Wiremock, temporary Git repositories and GPUI test
contexts. Use fake credentials and Claude processes; never run acceptance tests against live
approvals or pushes. Keep pixel comparisons separate from behavior assertions.

Syntax reference: [Cucumber Gherkin](https://cucumber.io/docs/gherkin/reference/).

# Existing tests and future bindings

These are implementation entry points for automation, not a claim that `.feature` files
already run. The Gherkin validator checks documents only. Existing tests provide **partial**
coverage: review every scenario's preconditions and outcomes before mapping its stable ID
to a test or Cucumber binding.

| Contract area | Existing test entry points |
| --- | --- |
| Projects | [Project UI interactions](../crates/mrde-app/src/ui/tests/projects.rs), [project persistence](../crates/mrde-app/src/settings/projects_tests.rs) |
| Connection settings | [UI tests](../crates/mrde-app/src/ui/tests.rs): `click_test_shows_busy_retains_token_and_never_saves`, `failed_save_keeps_form_and_displays_keychain_error`; [credential tests](../crates/mrde-store/src/secrets/tests.rs) |
| Inbox | [Core scoring](../crates/mrde-core/src/scoring.rs), [inbox filters](../crates/mrde-core/src/inbox.rs), [search/context UI](../crates/mrde-app/src/ui/tests/chrome.rs), [GitLab mocks](../crates/mrde-forge/src/gitlab.rs), [GitHub mocks](../crates/mrde-forge/src/github/tests.rs) |
| Reader and images | [Scrolling regressions](../crates/mrde-app/src/ui/tests/scrolling.rs), [Markdown layouts](../crates/mrde-app/src/markdown_layout_tests.rs), [UI tests](../crates/mrde-app/src/ui/tests.rs) |
| Review and chat | [Review orchestration](../crates/mrde-core/src/review.rs), [Claude parsing](../crates/mrde-claude/src/lib.rs), [Questions threads](../crates/mrde-app/src/ui/tests/ask.rs) |
| Approval | [UI tests](../crates/mrde-app/src/ui/tests.rs): `approval_requires_confirmation_and_sends_the_displayed_sha`; [GitHub mocks](../crates/mrde-forge/src/github/tests.rs): `approval_requires_the_reviewed_head_and_sends_that_commit` |
| Analysis | [Insights](../crates/mrde-insights/src/lib.rs), [UI tests](../crates/mrde-app/src/ui/tests.rs): `analysis_of_another_commit_is_never_displayed`, `tour_and_screenshots_run_only_on_click_and_only_when_configured` |
| Risk radar | [Scorers and temporary Git integration](../crates/mrde-risk/src/tests.rs), [normalization/median/verdict](../crates/mrde-core/src/risk.rs), [risk UI](../crates/mrde-app/src/ui/tests/risk.rs) |
| Resolver | [State machine](../crates/mrde-resolver/src/tests.rs), [UI tests](../crates/mrde-app/src/ui/tests.rs): `persisted_active_threads_require_manual_resume_after_relaunch` |
| Worktrees | [Git tests](../crates/mrde-git/src/lib.rs), [UI tests](../crates/mrde-app/src/ui/tests.rs): `worktrees_page_lists_worktrees_and_cleans_only_clean_ones` |
| UI contract | [Chrome layouts](../crates/mrde-app/src/ui/tests/chrome.rs), [scrolling](../crates/mrde-app/src/ui/tests/scrolling.rs), [native story catalog](ui/catalog.toml) |

## Automation sequence

1. Bind deterministic domain contracts: queue order, normalization, verdicts and attempt limits.
2. Add mocked provider and filesystem contracts, with isolated state per scenario.
3. Bind GPUI interactions to the existing test platform and meaningful layout assertions.
4. Review native captures on macOS; add visual comparisons only against explicitly reviewed
   references from a controlled font, display scale and window-size environment.

Loading, failure, cancellation and rapid-navigation states need interaction tests; static
screenshots cannot establish that the interface responds while a job is pending. Native
Touch ID, real provider permissions and terminal input remain separate manual checks.

@contract @radar_cli
Feature: Measure risk outside the desktop application
  The CLI and Claude plugin use the existing risk engine without a GPUI window or forge credentials.

  @MRDE-CLI-001 @smoke
  Scenario: Measure current edits without changing the repository
    Given a repository has staged, unstaged, deleted and non-ignored new files
    When I run the default radar scan
    Then all those changes are measured against HEAD
    And the original index, refs, objects and working files are unchanged
    And ignored files are excluded except the configured coverage report

  @MRDE-CLI-002
  Scenario: Compare committed revisions with the shared engine
    Given two locally available commits and additional uncommitted edits
    When I scan with an explicit base and head
    Then scores match the shared engine for those commits and configuration
    And uncommitted edits do not enter the report
    And the repository configuration comes from the analyzed head

  @MRDE-CLI-003
  Scenario: Fail on a missing local reference
    Given the requested base is not available locally
    When I run the radar scan
    Then a useful error is written to stderr with exit code 1
    And no Git fetch is attempted
    And stdout contains no partial JSON report

  @MRDE-CLI-004
  Scenario: Keep external checks opt-in
    Given risk configuration defines complexity and lint commands
    When I run a scan without Run checks
    Then no external check command is executed
    And those axes explain why they are unavailable
    And other measurements remain available

  @MRDE-CLI-005
  Scenario: Approve one exact proposed command for one invocation
    Given repository risk.toml proposes an untrusted lint command
    When I enable Run checks and approve that command's current fingerprint
    Then the command runs through the bounded checks executor in disposable snapshots
    And the approval is not persisted for subsequent invocations
    And a changed fingerprint cannot authorize another command

  @MRDE-CLI-006
  Scenario: Produce versioned machine-readable output
    Given a scan finishes with some unavailable axes
    When I request JSON output
    Then stdout contains one schema-versioned report with scores, evidence and input identity
    And the summary is marked partial with the number of measured axes
    And exit code 0 means a report was produced, not that risk is low
    And no team median is fabricated

  @MRDE-CLI-007 @visual
  Scenario: Export a partial radar
    Given one or more axes are unavailable
    When I request SVG output
    Then the document is valid SVG with the nine axis labels
    And unavailable axes have no plotted zero-risk point
    And the measured-axis count and missing team median are explicit

  @MRDE-CLI-008
  Scenario: Isolate a coverage report failure
    Given the configured coverage report is malformed or unreadable
    When I scan the current files
    Then the test-gap axis reports n/a
    And other axes still produce their measurements

  @MRDE-CLI-009
  Scenario: Run from a linked worktree
    Given a linked worktree has changes absent from the main checkout
    When I scan that worktree
    Then its own files and HEAD determine the result
    And the main checkout remains unchanged

  @MRDE-CLI-010
  Scenario: Invoke the radar from the current Claude conversation
    Given the MR.DE plugin is loaded and mrde-radar is installed
    When I explicitly invoke /mrde:radar
    Then Claude requests a local JSON report in the current repository
    And the explanation distinguishes measurements, estimates and interpretation
    And no second Claude session or forge request is started
    And loading the plugin alone does not run a scan

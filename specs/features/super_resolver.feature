@contract @resolver
Feature: Resolve a GitLab request through a bounded repair session
  Starting Super Resolver authorizes source-branch repairs and pushes, never approval or merge.

  @MRDE-RESOLVE-001
  Scenario: Rebase, repair and follow the resulting pipeline
    Given a selected GitLab request needs a rebase and has conflicts
    When I start Super Resolver
    Then it uses a separate writable worktree and asks Claude to repair the conflicts
    And it publishes with a lease on the observed source commit
    And it follows the pipeline for the resulting commit

  @MRDE-RESOLVE-002
  Scenario: Repair a failing pipeline and verify the new commit
    Given the pipeline for the current resolver commit has required failed jobs
    When Super Resolver inspects those failures
    Then their bounded logs are supplied as context for a correction
    And the corrected commit's pipeline must succeed before the run is marked successful
    And a successful pipeline for an older commit does not complete the run

  @MRDE-RESOLVE-003 @regression
  Scenario: Stop after three Claude repair attempts
    Given conflicts and pipeline failures have consumed three Claude attempts in this run
    When the request still needs a correction
    Then the thread enters Needs your input
    And no fourth automatic attempt starts

  @MRDE-RESOLVE-004
  Scenario: Hand over a blocking question to the user
    Given Claude asks a question that prevents the resolver from continuing
    When I take over the thread
    Then automation stops before its session opens in the embedded terminal
    And I can answer in that same session
    And another bounded run starts only after explicit Resume

  @MRDE-RESOLVE-005 @security
  Scenario: Preserve another person's source-branch update
    Given the remote source branch changes after the resolver observed it
    When the resolver attempts to publish a correction
    Then the lease check refuses to overwrite the newer remote commit
    And the thread requests intervention

  @MRDE-RESOLVE-006 @regression
  Scenario: Resume an interrupted session only on request
    Given an active resolver session was persisted before the application closed
    When I launch the application again
    Then that session is marked Interrupted
    And automation does not resume until I request it

  @MRDE-RESOLVE-007
  Scenario: Keep review navigation available during resolution
    Given a resolver is running for request A
    When I open request B or another project
    Then navigation remains available
    And the session and its results remain associated with A's original project

  @MRDE-RESOLVE-008
  Scenario: Report an unsupported provider
    Given the selected request belongs to GitHub
    When I try to start Super Resolver
    Then I see that Super Resolver is not supported for this provider
    And no repair or source-branch push starts

@contract @analysis
Feature: Understand changes using local evidence and explicit checks
  Evidence supports review without claiming that unexecuted checks passed.

  @MRDE-ANALYSIS-001
  Scenario: Analyze code without Claude
    Given the selected request has a clean review worktree
    When I select Analyze
    Then structure, history and risk findings are available for that commit
    And no Claude process is launched by this analysis

  @MRDE-ANALYSIS-002 @regression
  Scenario: Do not display analysis for another commit
    Given cached analysis belongs to commit A
    When I open the request at commit B
    Then the analysis for A is not presented as analysis of B

  @MRDE-ANALYSIS-003
  Scenario: Replace estimated file statistics with exact counts
    Given file statistics are estimated from the forge diff
    When I select Compute statistics
    Then the disktree uses local Git added and removed line counts
    And the source of those counts is shown

  @MRDE-ANALYSIS-004
  Scenario: Generate a guided tour only on request
    Given a request has no cached guided tour
    When I select Generate tour
    Then Claude produces a reading sequence with priority and skippable files
    And opening Structure alone does not trigger that generation

  @MRDE-ANALYSIS-005
  Scenario: Require a specification folder for compliance review
    Given no specification folder is configured
    When I open the compliance controls
    Then I am told to configure a specification folder
    And no compliance review starts

  @MRDE-ANALYSIS-006
  Scenario: Trace specification requirements to code and tests
    Given a configured specification folder contains requirements for the linked ticket
    When I explicitly run Check compliance
    Then the report relates requirements to specification files, implementation and tests
    And implemented requirements without tests are distinguished from tested requirements

  @MRDE-ANALYSIS-007
  Scenario: Compare screenshots from both revisions
    Given my project has a configured screenshot command
    When I select Capture screenshots
    Then the command runs against isolated base and head worktrees
    And matching image names show before, after and difference views
    And opening Visual alone does not execute the command

@contract @risk
Feature: Inspect an explainable risk profile
  Eight axes run from 0 to 10 with higher values meaning more risk.
  Missing evidence stays unavailable and never becomes a zero-risk measurement.

  @MRDE-RISK-001 @smoke
  Scenario: Calculate risk explicitly
    Given a request has no cached risk profile
    When I select Calculate risk in its change-analysis workspace
    Then local measurements produce a profile for the selected base and head commits
    And no Claude session starts
    And score calculation itself does not fetch remote data

  @MRDE-RISK-002
  Scenario Outline: Isolate an unavailable scorer
    Given one scorer "<failure>" while the other seven succeed
    When I calculate risk
    Then that axis displays n/a with a reason
    And the seven successful axes are still shown
    And the mean excludes the unavailable axis and the verdict is marked partial

    Examples:
      | failure                       |
      | exceeds its timeout           |
      | has no configured lint tool   |
      | cannot find Lizard            |
      | receives an incomplete report |

  @MRDE-RISK-003
  Scenario Outline: Use the configured normalization curve
    Given the diff-size curve is 0:0, 50:2, 300:5, 1000:8, 3000:10
    When a diff contains <lines> added and removed lines
    Then its diff-size score is <score> out of 10

    Examples:
      | lines | score |
      | 0     | 0     |
      | 25    | 1     |
      | 300   | 5     |
      | 4000  | 10    |

  @MRDE-RISK-004
  Scenario Outline: Recommend review depth at the default thresholds
    Given the available axes have mean <mean> and maximum <maximum>
    When the review-depth verdict is calculated
    Then the recommended depth is "<depth>"

    Examples:
      | mean | maximum | depth    |
      | 2.0  | 4.0     | Quick    |
      | 3.5  | 5.0     | Standard |
      | 6.0  | 7.0     | Deep     |
      | 2.0  | 8.0     | Deep     |

  @MRDE-RISK-005
  Scenario: Explain a heuristic test-gap measurement
    Given no coverage report is available for the changed source files
    When I calculate risk
    Then the test-gap axis uses the changed test-file to source-file ratio
    And its details identify this as an estimate rather than measured coverage

  @MRDE-RISK-006
  Scenario: Compare with an explicitly requested team median
    Given the baseline sample size is 30 and some historical commits are absent locally
    When I select Update team median
    Then available profiles from the most recent merged requests in that repository contribute to per-axis medians
    And unavailable history is excluded rather than scored zero
    And the radar reports the sample count for each axis

  @MRDE-RISK-007 @visual
  Scenario: Inspect axis evidence and start a focused discussion
    Given a risk profile is displayed
    When I select an axis
    Then its measurement, limitations and responsible files or lines are shown
    And Claude starts only if I subsequently select Ask Claude about this axis

  @MRDE-RISK-008 @visual
  Scenario: Inspect ownership and history from a file
    Given the disktree contains a changed file
    When I select that file
    Then its recent commits, line ownership percentages and related risk findings are shown
    And this selection does not launch Claude

  @MRDE-RISK-009 @regression
  Scenario: Sort using current cached risk profiles
    Given the queue contains current profiles, stale profiles and unmeasured requests
    When I choose Risk sorting
    Then current measured requests are ordered by descending mean risk
    And stale or unmeasured requests come after them
    And sorting does not start new calculations

  @MRDE-RISK-010 @security
  Scenario: Require approval for a repository-proposed command
    Given repository risk.toml introduces a lint command not trusted by my local configuration
    When I calculate risk
    Then that command is shown as Proposed and is not executed
    And I can approve its exact definition with Allow and calculate

  @MRDE-RISK-011
  Scenario: Apply layered configuration without reusing an incompatible profile
    Given global, project and repository risk configurations override the same thresholds
    When I calculate risk
    Then project values override global values and repository values override project values
    And a profile from another project, base, head or configuration is not reused

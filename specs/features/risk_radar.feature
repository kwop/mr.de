@contract @risk
Feature: Inspect an explainable risk profile
  Nine axes run from 0 to 10 with higher values meaning more risk.
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
    Given one scorer "<failure>" while the other eight succeed
    When I calculate risk
    Then that axis displays n/a with a reason
    And the eight successful axes are still shown
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
    Then the test-gap axis counts added source lines without a related test change
    And changes to unrelated tests do not hide those lines
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
    And I can approve its exact definition with Allow, install and calculate

  @MRDE-RISK-011
  Scenario: Apply layered configuration without reusing an incompatible profile
    Given global, project and repository risk configurations override the same thresholds
    When I calculate risk
    Then project values override global values and repository values override project values
    And a profile from another project, base, head or configuration is not reused

  @MRDE-RISK-012 @regression
  Scenario: Detect lint from the code changed in a merge request
    Given a monorepo contains Dart, TypeScript and Go packages
    And the merge request changes only Dart and TypeScript code
    When I calculate risk without explicit lint configuration
    Then the reviewed commit's relevant manifests produce proposed lint commands
    And the Go package and arbitrary CI scripts are not executed
    And local uncommitted manifest edits do not change the proposals

  @MRDE-RISK-013 @regression @visual
  Scenario: Install supported tools explicitly
    Given Lizard is missing and a detected lint command is proposed
    When I select Detect and install tools
    Then Lizard is prepared in the managed cache and reused by later calculations
    And the lint command remains proposed until Allow, install and calculate
    And progress and setup results remain visible without blocking navigation
    And setup cancellation or failure leaves the affected axis unavailable
    And ordinary calculation and refresh do not download packages

  @MRDE-RISK-014 @regression
  Scenario Outline: Use the committed JavaScript package manager
    Given the changed code uses ESLint and a committed "<lockfile>"
    When I allow the detected lint command and install its tools
    Then MR.DE uses "<manager>" with a frozen lockfile and lifecycle scripts disabled
    And a workspace installs from its owning root
    And the source worktree and lockfile remain unchanged

    Examples:
      | lockfile          | manager |
      | bun.lock          | Bun     |
      | bun.lockb         | Bun     |
      | package-lock.json | npm     |

  @MRDE-RISK-015 @regression
  Scenario: Explain an ambiguous or unsupported JavaScript setup
    Given JavaScript lockfiles conflict or identify an unsupported package manager
    When I prepare the detected lint command
    Then MR.DE explains the package-manager issue without substituting npm
    And the other risk axes can still finish

  @MRDE-RISK-016 @regression
  Scenario: Recalculate with Bun after the preparation process has ended
    Given an approved Bun lint check has prepared both commits
    And the installed Bun version has no offline install option
    When I restart MR.DE and calculate risks for the same commits
    Then MR.DE restores independent copies of the prepared dependencies
    And no package installation or download is started
    And workspace links and the measured warning delta are preserved
    And changes made by a linter do not affect the next calculation

  @MRDE-RISK-017 @regression
  Scenario: Rebuild dependencies when their cached preparation is no longer valid
    Given a dependency cache entry is incomplete, damaged or no longer matches the commit, command or Bun version
    When I calculate risks
    Then unavailable dependencies leave the lint axis unavailable with a setup action
    And the other risk axes can still finish
    When I run "Detect and install tools"
    Then MR.DE publishes a complete replacement dependency cache

  @MRDE-RISK-018 @regression
  Scenario: Revalidate requirements for an old primary ticket
    Given the primary ticket was created 180 days ago and the merge request was created today
    When I calculate risk with the default thresholds
    Then Ticket age scores 8 and the verdict recommends a Deep review
    And the explanation asks me to revalidate requirements without declaring the ticket obsolete
    And missing ticket dates stay unavailable

  @MRDE-RISK-019 @regression
  Scenario: Reuse persisted historical profiles for the team median
    Given the team median and its historical profiles have been saved
    And the requested target tip differs from the Git merge base
    When I reopen the request and update its team median
    Then unchanged historical profiles are reused without running their checks again
    And only newly merged or changed commit profiles are measured
    And ticket age in historical samples stays fixed at merge time

  @MRDE-RISK-020 @regression
  Scenario: Isolate oversized files during coupling analysis
    Given a request changes a large asset, a large source file and a readable dependency manifest
    When I calculate risk
    Then the asset does not participate in coupling analysis
    And the readable manifest contributes to coupling
    And the omitted source is named in a partial lower-bound explanation

  @MRDE-RISK-021 @regression @visual
  Scenario: Navigate from the disktree to a file
    Given a changed file is below the reader viewport or beyond the displayed file rows
    When I click its disktree tile
    Then its history panel opens and scrolls into the reader viewport
    And clicking the same tile again keeps the panel open without reloading its history

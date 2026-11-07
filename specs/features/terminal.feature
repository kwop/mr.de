@contract @terminal
Feature: Review merge requests in an interactive terminal
  Ratatui presents the shared application services without requiring GPUI or Ghostty.

  @MRDE-TUI-001 @smoke
  Scenario: Explore the terminal demo without external effects
    Given I launch mrde-tui with --demo
    When I navigate the inbox, reader, files, radar, history and settings
    Then the terminal responds to keyboard and mouse navigation
    And provider label colors and white scoped value segments stay readable
    And data and review results come from synthetic fixtures
    And no credentials, forge request or Claude process are used

  @MRDE-TUI-002 @regression
  Scenario: Navigate while a refresh is pending
    Given the terminal is waiting for a queue refresh
    When I open settings or select another merge request
    Then input and rendering remain responsive
    And a completed refresh does not move me to another screen
    And a late detail or analysis result cannot replace another selection

  @MRDE-TUI-003
  Scenario: Reuse the same risk engine and caches
    Given a project and a merge request are selected in the terminal
    When I explicitly calculate its radar or update its median
    Then the shared services use the same worktree, measurements and caches as the desktop
    And missing measurements remain n/a
    And the verdict and measured-axis count remain visible in the compact radar
    And selecting an axis reveals its raw measurement, evidence and limitations
    And repository-proposed lint commands require explicit authorization

  @MRDE-TUI-004 @security
  Scenario: Approve only the displayed commit after confirmation
    Given the terminal displays a merge request at a known SHA
    When I press the approval key
    Then I see the merge request and exact SHA in a confirmation dialog
    And no approval is sent until I confirm
    And confirmation submits that displayed SHA through the shared forge service
    And the SHA and confirmation keys stay visible while long context scrolls

  @MRDE-TUI-005
  Scenario: Ask Claude from the terminal and return
    Given I am reading a merge request or a risk axis
    When I explicitly choose Ask Claude
    Then the shared service prepares the worktree and context
    And the terminal restores its normal input mode before launching Claude in plan mode
    And the conversation uses my information language
    And exiting Claude returns to the same terminal reader

  @MRDE-TUI-006 @regression
  Scenario: Reach the bottom of a long document after resizing
    Given a description or history exceeds the terminal height
    When I resize the terminal and press End
    Then the last wrapped line is visible
    And Home returns to the start
    And a small terminal offers a readable compact presentation

  @MRDE-TUI-007
  Scenario: Produce a reproducible terminal reference
    Given I launch mrde-tui with --demo and --snapshot
    When I select a screen, language and terminal dimensions
    Then the exported SVG is generated from the actual Ratatui buffer
    And it contains only synthetic data
    And a real-credential snapshot without --demo is rejected

  @MRDE-TUI-008 @security
  Scenario: Launch the provisioned terminal companion
    Given the desktop package uses a valid macOS provisioning profile
    When I launch its terminal companion through Contents/MacOS/mrde-tui
    Then macOS accepts the helper's own provisioned application bundle
    And it has the same Keychain access group as the desktop
    And packaging verifies a demo export without reading saved credentials

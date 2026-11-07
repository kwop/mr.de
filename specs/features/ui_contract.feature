@contract @ui @visual
Feature: Preserve the native review interface contract
  Product text is English; content from requests and issues retains its original language.
  UI review uses synthetic stories and validates interactions separately from screenshots.

  @MRDE-UI-001 @regression
  Scenario: Separate workspace navigation from queue controls
    Given the inbox is visible
    When I inspect its navigation and controls
    Then the left sidebar contains workspace menus, projects and label filters
    And role filters, sorting and pagination are available above the queue
    And opening a request exposes its review actions beside the reading workflow

  @MRDE-UI-002 @regression
  Scenario: Preserve readable hierarchy at the minimum window size
    Given a window 1100 points wide and a request with a long title and many labels
    When I open the reader
    Then its title wraps without overlapping metadata
    And panel boundaries remain visible
    And request actions and all reading sections remain reachable

  @MRDE-UI-003
  Scenario Outline: Adapt the review context panel to the available width
    Given the window is <width> points wide
    When I open the review context panel
    Then it uses the "<layout>" layout
    And I can close it without losing my reading position

    Examples:
      | width | layout  |
      | 1100  | overlay |
      | 1440  | docked  |

  @MRDE-UI-004
  Scenario: Explain status using more than color
    Given a review operation succeeds, fails or requires user input
    When its result is displayed
    Then text or an icon identifies the outcome in addition to its color
    And an unavailable measurement is distinguished from a successful zero-risk measurement

  @MRDE-UI-005 @regression
  Scenario: Keep settings usable with debug logs open
    Given Project settings is open at the minimum window size
    When I expand Debug logs after a failed connection test
    Then the error, Test connection and Save remain reachable
    And the logs contain no entered secrets

  @MRDE-UI-006
  Scenario: Browse the interface without live services
    Given I launch a catalog story in UI preview mode
    When I explore its synthetic request and settings
    Then no saved credential is read and no forge request is sent
    And simulated review actions do not start Claude

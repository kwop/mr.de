@contract @inbox
Feature: Triage the complete review queue
  Filtering and sorting operate on the full queue before it is paginated.

  @MRDE-INBOX-001 @smoke
  Scenario Outline: Show eligible requests once
    Given a "<provider>" project with open, closed and draft requests
    And one open non-draft request matches both my reviewer and assignee roles
    When I refresh with Reviewer and Assignee selected
    Then only open requests matching a selected role are listed
    And draft requests have a visible "Draft" badge
    And the request matching both roles appears once

    Examples:
      | provider |
      | GitLab   |
      | GitHub   |

  @MRDE-INBOX-002
  Scenario: Sort numeric priorities before other signals
    Given requests carry priorities P10, P2, P::0 and no priority
    When I choose Priority sorting
    Then their priority order is P::0, P2, P10, then no priority
    And unrelated labels do not change their priority

  @MRDE-INBOX-003 @regression
  Scenario: Paginate after global sorting
    Given 26 eligible requests and the highest-priority request was fetched last
    When I choose Priority sorting and open the first page
    Then the highest-priority request is first
    And the first page contains 25 requests
    And the next page contains the remaining request

  @MRDE-INBOX-004
  Scenario: Combine text and label filters
    Given the queue contains requests from several projects with different labels
    When I enter search words and select two labels
    Then every visible request matches every search word and both labels
    And matching text ignores letter case
    And pagination and adjacent-request navigation follow the filtered list

  @MRDE-INBOX-005
  Scenario: Rank requests whose size is unknown last
    Given requests with known and unknown diff sizes
    When I choose Smallest diff
    Then known sizes are ordered from smallest to largest
    And unknown sizes follow all known sizes

  @MRDE-INBOX-006 @regression
  Scenario: Keep navigation responsive during refresh
    Given a queue refresh is waiting for the provider
    When I open Project settings and edit its display name
    Then the settings page opens and accepts my input before the refresh completes
    And completion of the refresh does not navigate me away from settings

  @MRDE-INBOX-007
  Scenario: Refresh periodically without starting analysis
    Given a connected project with a configured refresh interval
    When the refresh interval expires
    Then the queue is refreshed in the background
    And cached rows remain available while loading
    And no Claude session or local risk calculation starts automatically

  @MRDE-INBOX-008 @regression
  Scenario: Preserve a newer sorting choice after a late refresh
    Given a refresh started with Priority sorting
    When I change sorting to Newest before that refresh completes
    Then the displayed queue follows Newest sorting
    And the older refresh cannot restore Priority sorting

  @MRDE-INBOX-009
  Scenario: Use labels from the primary ticket
    Given a request implements one ticket and merely mentions another issue
    And the implemented ticket has a P::1 label
    When I view or sort the request
    Then its labels include those of the implemented ticket
    And unrelated issue labels do not influence its priority

  @MRDE-INBOX-010 @regression @visual
  Scenario: Render scoped labels with their provider color
    Given a request has the label "Review::Dev" with color "#426A9A"
    When I view the queue or request details
    Then the scope "Review" uses the provider color
    And the value "Dev" uses an adjoining white segment with readable text

  @MRDE-INBOX-011 @regression
  Scenario: Sort by approval count without treating unknown counts as zero
    Given requests with zero, one, two and unknown approval counts
    When I choose Approvals sorting
    Then requests are ordered by zero, one, two and unknown approvals across all pages
    And equal counts use priority as the tie breaker
    And the selected project remembers this sorting choice
    And no provider request is made just to change the sort

  @MRDE-INBOX-012
  Scenario: Reorder the queue after approving a request
    Given I use Approvals sorting and read a request with zero approvals
    When I confirm its approval and the provider reports the new count
    Then the request moves to its new position in the queue
    And the reader stays on that request

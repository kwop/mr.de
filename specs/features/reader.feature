@contract @reader
Feature: Read a request without losing review context
  The reader presents the description and files while navigation and actions stay available.

  @MRDE-READ-001 @smoke
  Scenario: Open and focus a request
    Given the review queue is visible
    When I select a request and focus the reader
    Then the queue is hidden and the selected request remains open
    And Show queue restores the queue without changing the selection

  @MRDE-READ-002 @regression
  Scenario: Ignore an earlier request's late details
    Given the details of request A are still loading
    When I select request B and its details arrive first
    Then B remains selected when the response for A arrives

  @MRDE-READ-003 @regression @visual
  Scenario: Render mixed Markdown without overlapping blocks
    Given a description contains headings, paragraphs, lists, a table and a fenced code block
    When I open the description
    Then the Markdown structure is rendered
    And code uses the code font in a distinct block
    And code, tables and adjacent paragraphs do not overlap

  @MRDE-READ-004 @regression
  Scenario: Browse changed files without expanding every diff
    Given a request changes many files
    When I open its file list
    Then I see file paths and available change counts
    And large inline diffs are not expanded for every file
    And I can reach the final file in the list

  @MRDE-READ-005 @regression @visual
  Scenario Outline: Reach the bottom of a long reading page
    Given the "<page>" page contains more content than fits in the reader
    When I scroll the reader to the bottom
    Then its final item is fully reachable
    And pinned review actions remain accessible

    Examples:
      | page        |
      | Description |
      | History     |
      | Changes     |

  @MRDE-READ-006
  Scenario: Open a request outside the current queue
    Given a valid request link belongs to the configured instance but is outside the queue
    When I submit it through Open MR
    Then that request opens in the reader
    And an unchanged queue refresh does not close it

  @MRDE-READ-007 @security
  Scenario: Reject a link belonging to another instance
    Given the active project uses "https://gitlab.example.com"
    When I submit a request link from "https://other.example.com"
    Then I see an instance mismatch error before a request is sent
    And no credential is sent to the other instance

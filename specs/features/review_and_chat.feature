@contract @claude
Feature: Request Claude assistance explicitly
  Reviewing or discussing code is a deliberate action bound to the selected commit.

  @MRDE-AI-001 @smoke
  Scenario: Read a request without starting Claude
    Given Claude Code is available and the request has no cached pre-review
    When I open the request
    Then its description can be read without launching Claude

  @MRDE-AI-002
  Scenario Outline: Run the selected review depth
    Given a request at commit A has no cached review at depth "<depth>"
    When I select "<depth>" and run the review
    Then Claude reviews commit A in its dedicated review worktree
    And the result shows summary, intention, severity-ranked risks, priority files and confidence

    Examples:
      | depth    |
      | Quick    |
      | Standard |
      | Deep     |

  @MRDE-AI-003
  Scenario: Reuse the cached review for the same commit and depth
    Given commit A has a cached Standard review
    When I request a Standard review of commit A again
    Then the cached result is displayed without another Claude run
    And it is not reused for a different head or base commit

  @MRDE-AI-004
  Scenario: Reject a malformed Claude response
    Given Claude returns a response that cannot be parsed as a valid review
    When the run finishes
    Then an actionable error is displayed
    And the invalid response is not presented as a successful review

  @MRDE-AI-005 @regression
  Scenario: Keep navigation responsive during a review
    Given a pre-review of request A is running
    When I open request B or Project settings
    Then navigation completes without waiting for Claude
    And the result for A does not replace B's content or the settings page

  @MRDE-AI-006 @smoke
  Scenario: Discuss the selected request in an embedded thread
    Given a request has a description, a linked ticket and a cached pre-review
    When I select Ask Claude
    Then a Questions thread opens with that request's context
    And Claude starts in read-only mode in the request's worktree
    And I can interact through the embedded terminal

  @MRDE-AI-007
  Scenario: Resume a discussion for the same commit
    Given a Questions thread already exists for request A at commit X
    When I select Ask Claude for that same request and commit
    Then the existing session is reused
    And a different commit receives a separate thread

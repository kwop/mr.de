@contract @approval
Feature: Approve exactly the reviewed commit
  Approval is explicit and does not merge the request.

  @MRDE-APPROVE-001 @smoke
  Scenario Outline: Confirm approval for the displayed commit
    Given a "<provider>" request is displayed at commit A
    When I select Approve
    Then a confirmation identifies the request and commit A
    And no approval is sent until I confirm

    Examples:
      | provider |
      | GitLab   |
      | GitHub   |

  @MRDE-APPROVE-002
  Scenario: Cancel approval
    Given the approval confirmation is open
    When I cancel
    Then no approval request is sent
    And the reader remains on the same request

  @MRDE-APPROVE-003 @regression
  Scenario: Refuse approval of a commit that changed remotely
    Given I reviewed commit A and the remote request now points to commit B
    When I confirm approval for commit A
    Then the operation refuses to approve the unseen commit B
    And I see an error that allows me to refresh and review again

  @MRDE-APPROVE-004
  Scenario: Report a completed approval
    Given the provider accepts approval for the reviewed commit
    When the approval completes
    Then I see an explicit success result
    And the request is not merged by MR.DE

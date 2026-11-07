@contract @worktrees
Feature: Manage local review worktrees without losing work
  Review worktrees are scoped to the project and commit and use the user's SSH setup.

  @MRDE-TREE-001
  Scenario: Reuse the matching clean review worktree
    Given the selected request already has a clean worktree for its commit
    When I run a local analysis for that commit
    Then that worktree is reused
    And the user's main checkout is not modified

  @MRDE-TREE-002
  Scenario: Remove a clean worktree on request
    Given the Worktrees page lists a clean review worktree
    When I select Clean for that worktree
    Then that worktree is removed
    And the bare repository and other review worktrees are retained

  @MRDE-TREE-003 @regression
  Scenario Outline: Refuse cleanup when local files would be lost
    Given a review worktree contains "<changes>"
    When I try to clean it
    Then cleanup is refused with a visible reason
    And all local files are retained

    Examples:
      | changes                |
      | modified tracked files |
      | untracked files        |
      | ignored files          |

  @MRDE-TREE-004
  Scenario: Keep repair worktrees outside review cleanup
    Given a resolver thread owns a writable worktree
    When I browse and clean review worktrees
    Then the resolver worktree and its manual changes are retained

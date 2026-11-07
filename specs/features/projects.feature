@contract @projects
Feature: Independent review projects
  A project connects one provider and keeps its configuration, credentials and review data separate.

  @MRDE-PROJ-001 @smoke
  Scenario Outline: Create another project without replacing the existing connection
    Given a saved GitLab project named "Platform"
    When I save a new "<provider>" project named "Community"
    Then both projects are available in the project selector
    And the Platform configuration is unchanged

    Examples:
      | provider |
      | GitLab   |
      | GitHub   |

  @MRDE-PROJ-002 @regression
  Scenario: Switching projects rejects late results from the previous project
    Given Platform has a pending queue refresh and an open merge request
    When I switch to Community before the refresh completes
    Then the reader, images and secret inputs from Platform are cleared
    And a late Platform response cannot replace Community content

  @MRDE-PROJ-003
  Scenario: Two accounts on one instance remain independent
    Given Platform and Community use different accounts on the same forge instance
    When I save new credentials for Community
    Then Platform still uses its own credentials
    And cached reviews and threads remain scoped to their originating project

  @MRDE-PROJ-004
  Scenario: Rename a project without changing its identity
    Given a saved project with reviews, credentials and worktrees
    When I change its display name and save
    Then the existing credentials, reviews and worktrees are still available

  @MRDE-PROJ-005
  Scenario: Load non-secret configuration from TOML
    Given a valid projects.toml defines Platform and Community with Community active
    When I launch the application
    Then Community is selected
    And its configured provider, scope, refresh interval and review preferences are used

  @MRDE-PROJ-006
  Scenario: Import the legacy single connection
    Given a legacy config.toml exists and no projects.toml exists
    When I launch the application and save the imported project
    Then the existing connection is available as the Default project
    And the existing credentials remain usable
    And the legacy file is retained

  @MRDE-PROJ-007
  Scenario: Reject ambiguous project configuration
    Given projects.toml contains duplicate project identifiers
    When I load the project catalog
    Then a configuration error identifies the duplicate
    And no project configuration is silently overwritten

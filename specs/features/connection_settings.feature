@contract @settings
Feature: Test and save connection settings independently
  Settings give explicit progress and outcomes while preserving entered values.

  @MRDE-CONN-001 @regression @smoke
  Scenario: Test the connection without saving
    Given I have entered a provider URL and a masked token
    And the connection service has not answered yet
    When I select Test connection
    Then a progress indicator is visible until the request completes
    And the entered token remains masked and present
    And no configuration or credential is saved by the test

  @MRDE-CONN-002
  Scenario: Show a successful connection test
    Given my entered credentials and configured scope are accepted by the provider
    When the connection test completes
    Then I see an explicit success result
    And Save remains a separate action

  @MRDE-CONN-003 @regression
  Scenario: Retry a failed connection test without re-entering secrets
    Given the provider rejects the entered credentials
    When the connection test completes
    Then I see a useful error with secrets redacted
    And the entered values remain available for correction and retry

  @MRDE-CONN-004 @smoke
  Scenario: Save independently of network availability
    Given the configuration is valid and the credential store accepts writes
    And the forge cannot be reached
    When I select Save
    Then the configuration and credentials are saved without testing the connection
    And I see a save confirmation while remaining in settings
    And the token input remains masked and present

  @MRDE-CONN-005 @regression
  Scenario: Preserve the form when credential storage fails
    Given the credential store rejects a save
    When I select Save
    Then I see the credential storage error
    And the entered form values remain available
    And the previous saved credentials remain usable

  @MRDE-CONN-006
  Scenario: Keep saved secrets when their inputs are blank
    Given the active project already has a saved token and Cloudflare credentials
    When I save a changed refresh interval with blank secret inputs
    Then those saved secrets are retained for the active project

  @MRDE-CONN-007
  Scenario: Explicitly remove Cloudflare credentials
    Given the active GitLab project has saved Cloudflare credentials
    When I choose Remove saved Cloudflare credentials and save
    Then subsequent requests for this project omit the Cloudflare credential headers
    And its GitLab token is retained

  @MRDE-CONN-008 @security
  Scenario: Use the configured GitLab and Cloudflare credentials together
    Given the active GitLab project requires a Cloudflare service token
    When I test the connection
    Then the configured GitLab and Cloudflare credentials accompany the request to that instance
    And credentials are absent from TOML, SQLite, debug logs and Claude context

  @MRDE-CONN-009 @security
  Scenario: Respect cancelled system authentication
    Given a protected credential read requires system authentication
    When I cancel that authentication
    Then I see an authentication failure
    And the application does not bypass it by using legacy credentials

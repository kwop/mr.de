@contract @languages
Feature: Choose interface and information languages independently
  Presentation preferences are global and shared by the desktop and terminal frontends.
  Original repository content, identifiers, commands and JSON schema keys remain unchanged.

  @MRDE-LANG-001 @smoke
  Scenario: Use French controls with English Claude responses
    Given the interface language is French and the information language is English
    When I open the inbox and run a review explicitly
    Then application controls are displayed in French
    And Claude is instructed to write explanations in English
    And merge request titles and descriptions retain their source text

  @MRDE-LANG-002 @security
  Scenario: Save languages without touching credentials
    Given I am on the Languages settings page
    And the forge is unreachable and credential access is unavailable
    When I select French for the interface and save the languages
    Then the preferences are saved in projects.toml without a network or credential operation
    And switching projects or restarting either frontend retains both preferences

  @MRDE-LANG-003 @regression
  Scenario: Do not restore an AI result in the wrong language
    Given a review is cached in English for the displayed commit
    When I save French as the information language
    Then the old review is removed from the reader without starting Claude
    And a subsequent explicit review uses a different language-specific cache entry
    And numeric risk measurements and the team median remain cached

  @MRDE-LANG-004 @regression
  Scenario: Keep preferences unchanged after a failed save
    Given the saved interface language is English
    And the configuration file cannot be written
    When I choose French and save the languages
    Then the interface retains the previously saved language
    And the draft remains available with a visible failure

  @MRDE-LANG-005
  Scenario: Read a legacy project catalog
    Given projects.toml has no language preferences
    When either frontend loads it
    Then both languages default to English
    And credential namespaces and project scopes remain unchanged

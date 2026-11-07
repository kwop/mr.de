@contract @images
Feature: Inspect description images at a useful scale
  The viewer starts fitted to the window and permits scrolling when magnified.

  @MRDE-IMAGE-001 @regression @visual @smoke
  Scenario: Fit a tall image inside the available window
    Given the description contains a portrait image taller than the window
    When I click the image
    Then the entire image fits inside the viewer below its toolbar
    And its aspect ratio is preserved
    And the close and zoom controls are visible

  @MRDE-IMAGE-002 @regression @visual
  Scenario: Zoom and reach every image edge
    Given an image is open in the viewer
    When I magnify it until it exceeds the viewport in both dimensions
    Then I can scroll horizontally and vertically to all image edges
    And Fit restores the complete image inside the window

  @MRDE-IMAGE-003
  Scenario: Return to the description after inspection
    Given the image viewer was opened from a scrolled description
    When I press Escape
    Then the viewer closes
    And the description retains its reading position

  @MRDE-IMAGE-004 @security
  Scenario: Resolve a private GitLab upload through its project
    Given a GitLab description references a relative upload path
    When its image is loaded
    Then the image is requested through the configured project's authenticated upload endpoint
    And credentials are not forwarded to unrelated public image hosts

  @MRDE-IMAGE-005 @regression
  Scenario: Keep the viewer usable after an image download failure
    Given the referenced image cannot be downloaded
    When I open it
    Then an error is visible
    And I can close the viewer and continue reading

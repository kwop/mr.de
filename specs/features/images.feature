@contract @images
Feature: Inspect description images at a useful scale
  The viewer starts centered and fitted to the window, with click zoom and grab panning.

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

  @MRDE-IMAGE-006 @regression @visual
  Scenario: Zoom at the point clicked and cycle back to the fitted image
    Given a decoded image is centered and fitted to the viewer
    When I hover over the image
    Then a plus magnifier replaces the arrow cursor
    When I click a detail repeatedly
    Then the image zooms to 2, 4 and 8 times the fitted size around that point
    And the magnifier becomes a minus at the maximum zoom
    When I click once more
    Then the complete image is centered and fitted to the window again

  @MRDE-IMAGE-007 @regression
  Scenario: Grab a magnified image without accidentally zooming it
    Given an image exceeds the viewport in both dimensions
    When I hold the left mouse button and move the pointer
    Then a grabbing hand follows the gesture and the image moves with it
    And panning stops at the image edges
    When I release the button inside or outside the viewer
    Then the zoom level stays unchanged and the grab ends
    And later pointer movement without a pressed button does not move the image

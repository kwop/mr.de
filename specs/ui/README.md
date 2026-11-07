# MR.DE UI reference

This is the working UI/UX standard for new development: the product contracts, existing
native components, reproducible screen states and a review procedure. Captures are candidates
until reviewed; there is currently no approved pixel baseline or automated image-diff gate.

## Native Storybook equivalent

GPUI Component's upstream project, now [GPUI Kit](https://github.com/longbridge/gpui-kit#desktop-gallery-story),
provides a native **Story** component gallery. MR.DE currently pins GPUI 0.2.2 and vendors
GPUI Component 0.5.0. The upstream gallery demonstrates widgets; it is not MR.DE's design
system and adding it does not test our screens. No framework upgrade is required here.

MR.DE's [catalog](catalog.toml) launches the actual application with `--ui-preview`, its
existing fake executor and synthetic fixtures. This is a screen catalog, not a separate
implementation of the interface and not a webview. Each story links product contract IDs
and names what a reviewer should inspect.

```sh
python3 scripts/ui_catalog.py list
python3 scripts/ui_catalog.py open risk.partial
python3 scripts/ui_catalog.py open image.zoom
python3 scripts/ui_catalog.py capture reader.markdown reader.history-bottom image.fit image.zoom risk.partial
```

`open` is interactive; close the preview window to return to the terminal. `capture` builds
once and visits states sequentially, saving PNGs, logs, a metadata manifest and an HTML
contact sheet in a new temporary directory. Pass `--output /tmp/mrde-review-001` to choose a
new directory; an existing directory is refused. `--no-build` reuses the local debug binary.
The HTML displays native captures; it is not an alternate renderer of MR.DE.

The launcher clears inherited `MRDE_*` overrides, including real-data capture and private
risk-report inputs. Preview actions use fakes. Capture failure or timeout returns a nonzero
exit status, even when the application exits without writing a PNG.

## Visual language

[theme.rs](../../crates/mrde-app/src/theme.rs) is authoritative for values; reuse its constants
and the existing widgets rather than copying styles into each new panel.

| Role | Current reference |
| --- | --- |
| Background hierarchy | Canvas `#141518`, sidebar `#1c1e22`, queue `#202328`, reader `#292c32` |
| Text | Primary `#eef0f4`, heading `#f5f6f8`, secondary `#bbc2ce` |
| Boundaries and selection | Panel border `#535965`, selected row `#34445e` |
| Semantic accents | Success `#6bcc91`, error `#f08080`, warning `#efb85d`, information `#8ab4f8` |
| Typography | macOS system UI, 14-point base; Menlo, 12-point code |
| Geometry | 10-point sheets, 6-point cards/controls, 3-point chips; spacing from existing GPUI helpers |
| Icons | Existing GPUI Component/Lucide icons, consistent sizing and alignment |

Keep the logo quiet. Preserve provider label colors; scoped labels have an adjoining white
value segment with readable text. Use wrapping paragraphs for titles and deliberate ellipsis
for single-line paths. Code blocks, tables and prose need separate layout bounds and spacing.
Do not replace lists or icons with arbitrary Unicode glyphs.

The current application is dark. Component-level palette checks do not establish
whole-application light-theme support.

## Interaction and measurable review criteria

- Navigation, search and settings input must work while a mocked network request or review
  is deliberately held pending. Test completion must not move the user to another page.
- Test and Save remain separate. Loading, success, error and unavailable states need visible
  text or an icon; preserve inputs after failure and expose an appropriate retry action.
- Verify the minimum 1100-point width and both 720- and 900-point heights. Scroll every long
  page to its final item; pinned controls and image-viewer controls must remain reachable.
- At 1440 points the context panel can dock; at 1100 it overlays. Use GPUI `simulate_resize`
  for exact breakpoints: macOS may override a requested capture window size. The capture
  manifest records requested size and actual PNG pixels; these are different units.
- Adopt contrast targets of 4.5:1 for normal text and 3:1 for essential non-text indicators
  against adjacent colors, following [W3C text contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)
  and [non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html).
  These are review targets, not a claim that the existing interface has passed an accessibility audit.
- Inspect long titles, scoped labels, missing values, large files, tall images and partial
  radar profiles. Exercise focus, Escape, scrolling, selection and error recovery; a clean
  screenshot alone does not establish a usable interaction.

## Reviewing a change

1. Identify the Gherkin IDs affected by the change; update the contract when behavior changes.
2. Open the relevant stories and run the existing Rust interaction tests for those paths.
3. Capture before and after with the same fixture, fonts, scale, viewport and scroll state.
4. Review overflow, hierarchy, affordances and the story's inspection notes. Record observed
   regressions separately from intentional changes; never approve a capture automatically.
5. When a reference is accepted, record its source commit, story ID, platform, dimensions,
   reviewer and intentional exceptions beside the chosen images. Store only generic demo data.

For a new component, add default, focused, disabled, loading, empty, error and success states
where applicable. The present catalog covers existing stable previews; transient states are
currently exercised by [GPUI interaction tests](../../crates/mrde-app/src/ui/tests.rs), not
frozen as screenshot stories. Add stable fixtures before introducing pixel regression tests.

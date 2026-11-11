---
name: Asset Hub
description: Find a Unity asset on GitHub, judge it on a calibrated stage, export it with its dependencies.
colors:
  checker-orange: "#d67e2c"
  checker-orange-hover: "#e08a3a"
  on-orange: "#1a1208"
  graphite-ground: "#1f1f20"
  graphite-panel: "#2a2a2b"
  graphite-raise: "#343435"
  hairline: "#3a3a3c"
  hairline-strong: "#4b4b4e"
  stage-grey: "#767676"
  paper-text: "#e9e9e6"
  text-secondary: "#c9c9c5"
  text-mute: "#a3a3a0"
  text-dim: "#94948f"
  decoration-faint: "#7a7a76"
  state-ok: "#7cc47a"
  state-warn: "#e6b84a"
  state-danger: "#ee7a80"
  state-info: "#8fa8c8"
  type-fbx: "#6a76c4"
  type-texture: "#e0a32e"
  type-material: "#67bdaa"
  type-audio: "#c15a63"
  type-prefab: "#627a9d"
  type-animation: "#9dbc40"
  type-font: "#8d67a3"
  type-vfx: "#bb5695"
  type-sprite: "#0885a1"
typography:
  title:
    fontFamily: "Geist, -apple-system, BlinkMacSystemFont, 'Apple SD Gothic Neo', 'Malgun Gothic', system-ui, sans-serif"
    fontSize: "16px"
    fontWeight: 600
    lineHeight: 1.25
  body-lg:
    fontFamily: "Geist, -apple-system, BlinkMacSystemFont, 'Apple SD Gothic Neo', 'Malgun Gothic', system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.4
  body:
    fontFamily: "Geist, -apple-system, BlinkMacSystemFont, 'Apple SD Gothic Neo', 'Malgun Gothic', system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.4
  label:
    fontFamily: "Geist, -apple-system, BlinkMacSystemFont, 'Apple SD Gothic Neo', 'Malgun Gothic', system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 500
    lineHeight: 1.4
    letterSpacing: "0.04em"
  measure:
    fontFamily: "'Geist Mono', ui-monospace, 'SF Mono', Menlo, monospace"
    fontSize: "11px"
    fontWeight: 400
    lineHeight: 1.4
    fontFeature: "tnum"
  measure-lg:
    fontFamily: "'Geist Mono', ui-monospace, 'SF Mono', Menlo, monospace"
    fontSize: "13px"
    fontWeight: 500
    lineHeight: 1.4
    fontFeature: "tnum"
rounded:
  sm: "4px"
  md: "6px"
  lg: "8px"
  pill: "999px"
spacing:
  sp-1: "4px"
  sp-2: "8px"
  sp-3: "12px"
  sp-4: "16px"
  sp-5: "24px"
  sp-6: "32px"
components:
  button-primary:
    backgroundColor: "{colors.checker-orange}"
    textColor: "{colors.on-orange}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "0 9px"
    height: "28px"
  button-primary-hover:
    backgroundColor: "{colors.checker-orange-hover}"
    textColor: "{colors.on-orange}"
  button-secondary:
    backgroundColor: "{colors.graphite-raise}"
    textColor: "{colors.paper-text}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "0 9px"
    height: "28px"
  button-signout:
    textColor: "{colors.state-info}"
    rounded: "{rounded.md}"
    height: "28px"
  icon-button:
    backgroundColor: "{colors.graphite-raise}"
    textColor: "{colors.text-mute}"
    rounded: "{rounded.md}"
    size: "28px"
  input:
    backgroundColor: "{colors.graphite-ground}"
    textColor: "{colors.paper-text}"
    rounded: "{rounded.md}"
    padding: "0 10px"
    height: "30px"
  sidebar-row:
    textColor: "{colors.text-mute}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "0 8px"
    height: "28px"
  sidebar-row-active:
    backgroundColor: "{colors.graphite-raise}"
    textColor: "{colors.paper-text}"
  filter-pill:
    backgroundColor: "{colors.graphite-raise}"
    textColor: "{colors.paper-text}"
    typography: "{typography.body}"
    rounded: "12px"
    padding: "0 7px 0 10px"
    height: "24px"
  type-chip:
    backgroundColor: "rgba(31, 31, 32, 0.88)"
    textColor: "{colors.paper-text}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: "0 6px 0 5px"
    height: "18px"
  asset-stage:
    backgroundColor: "{colors.stage-grey}"
    rounded: "{rounded.md}"
    width: "minmax(120px, 1fr)"
  raised-panel:
    backgroundColor: "{colors.graphite-panel}"
    rounded: "{rounded.lg}"
    padding: "8px"
---

# Design System: Asset Hub

## Overview

**Creative North Star: "The Calibration Bench"**

Asset Hub is a look-dev bench, not a showroom. Every thumbnail and viewer sits on the same true 18% grey stage with a soft floor shadow. An asset is judged in the same light wherever it appears. A graphite shell frames that stage and stays quiet: three close greys for ground, panel and raised surfaces, hairline borders, and warm off-white text. The calibration tools of a lighting artist are the visual source: the grey ball, the chrome ball and the colour checker.

Colour carries meaning only. The colour checker's orange patch marks the one primary action on a surface. The other checker patches mark asset types, as small swatches on chips and sidebar rows. Green, yellow and red mark state. Nothing else is coloured. The density is high and steady: a 48px top bar, a 232px sidebar, 28px rows, a grid of small cells (120px or 180px minimum) and a 380px inspector. People scan hundreds of assets at once and read the facts that decide.

The system rejects the dark-panel asset grid with huge cards, a neon accent, and decoration that carries no information. It also replaces the 2.0 look (gunmetal, amber accent, Chakra Petch display face, chest logo) in full.

**Key Characteristics:**
- One calibrated stage (18% grey) under every preview, cards and viewers alike.
- Graphite shell in three tonal steps, separated by 1px hairlines, not by shadow.
- One orange for the primary action. Colour-checker patches for asset types. Green, yellow and red for state.
- Geist for words, Geist Mono for figures and paths, four fixed size steps.
- Dense, keyboard-first rows at 28px. Quiet until hovered or pressed.

## Colors

A neutral graphite shell around a mid-grey stage, with colour reserved for action, type and state.

### Primary
- **Checker Orange** (`checker-orange`): the fill of the single primary action on a surface: Export, Download, Sign in, Send. Text on it is the dark Burnt Umber Ink (`on-orange`), because white text fails 4.5:1 on this orange.
- **Checker Orange, Lifted** (`checker-orange-hover`): the hover fill of the primary button only.
- **Burnt Umber Ink** (`on-orange`): text and icons on orange.

### Asset types (the colour checker)
- **Checker patches** (`type-fbx`, `type-texture`, `type-material`, `type-audio`, `type-prefab`, `type-animation`, `type-font`, `type-vfx`, `type-sprite`): one per asset type. They appear only as the 7–8px swatch on a type chip over a card and as the dot in a sidebar type row. They also appear as the patch row in the inspector's calibration strip. The type word stays in text colour. The model and font patches are lifted a little so they hold 3:1 on the panel.

### State
- **Checker Green** (`state-ok`): budget fits, new assets, copied, done, the live-watch dot.
- **Checker Yellow** (`state-warn`): warnings, over-budget verdicts, failed updates.
- **Soft Red** (`state-danger`): errors and over-limit meters only.
- **Calm Blue** (`state-info`): the sign-out button and informational dots. Signing out is a normal choice, so it never takes the danger red.
- State tints are mixed, not new hues: 14% of the state colour into the panel for a background, 40% for a border.

### Neutral
- **Graphite Ground** (`graphite-ground`): the app background, top bar, toolbars, fields and the calibration strip.
- **Graphite Panel** (`graphite-panel`): sidebar, inspector, export strip, menus, dialogs and the Settings sheet.
- **Graphite Raise** (`graphite-raise`): buttons, the pressed sidebar row, hovered menu rows and pills.
- **Hairline** (`hairline`) and **Strong Hairline** (`hairline-strong`): every border and divider. The strong one marks floating panels, hovered controls and focus-adjacent edges.
- **Stage Grey 18%** (`stage-grey`): the light table. Thumbnails, viewers, the export-list thumbnails and the brand mark sit on it. Cards add a soft radial lift (#7e7e7e at the top to #6c6c6c at the edges) around it.
- **Paper White** (`paper-text`): primary text and the focus ring.
- **Soft Paper** (`text-secondary`), **Mute Grey** (`text-mute`), **Dim Grey** (`text-dim`): secondary text, counts and placeholders. Each holds 4.5:1 on ground and panel. Dim Grey is used only on ground and panel.
- **Faint Grey** (`decoration-faint`): decoration only, such as the middle dot between card facts. Never for text a person must read.

### Named Rules
**The One Orange Rule.** Checker Orange is the fill of the single primary action on a surface. It is not a highlight, a selection colour or a pressed state. Selection and pressed states use Paper White, Graphite Raise and Ground.

**The Colour Means Something Rule.** Outside the primary action and state, colour comes only from the colour-checker type patches, and only as a small swatch. A coloured surface, border or text that names no type and no state is a defect.

**The Same Light Rule.** Every preview of an asset sits on Stage Grey. A dark backdrop is a viewer option the user picks, never a default.

## Typography

**Body Font:** Geist (with -apple-system, Apple SD Gothic Neo and Malgun Gothic for Korean, then system-ui)
**Label/Mono Font:** Geist Mono (with ui-monospace, SF Mono, Menlo)

**Character:** Geist is a plain, even grotesque that keeps a dense tool readable at 11–13px. Geist Mono with tabular figures makes sizes, counts and paths line up in columns. Both are bundled through @fontsource (400, 500 and 600 for Geist, 400 and 500 for Geist Mono). The page loads no remote font. Korean falls back to the system face.

### Hierarchy
- **Title** (600, 16px, 1.25): the toolbar heading ("FBX", "Texture"), the inspector asset name, the Settings and dialog headings, the sign-in title, lead and code cells.
- **Body Large** (400–500, 13px, 1.4): Settings body, dialog body, field labels, the export-list summary, menu notices and the brand name (600).
- **Body** (400, 12px, 1.4): the base size. Sidebar rows, buttons (500), card names (500), menu rows, pills and notes.
- **Label** (500, 11px, 0.04em tracking, sentence case): group labels over sidebar sections, filter groups and menus. Also chips on cards, fold summaries and fact captions.
- **Measure** (Geist Mono 400, 11px, tabular figures): counts, sizes, triangle counts, versions and paths. **Measure Large** (Geist Mono 500, 13px) holds the inspector fact values.

### Named Rules
**The Four Steps Rule.** Text uses exactly four sizes: 11, 12, 13 and 16px (`--fs-xs` to `--fs-lg`). There is no display size. Hierarchy comes from weight (400, 500, 600) and from colour (Paper White to Dim Grey), not from new sizes.

**The Mono Is Data Rule.** Geist Mono is for figures and machine strings: sizes, counts, times, versions, file paths and typed paths. Prose never takes the mono face. A path field whose empty hint is a sentence shows the hint in Geist.

## Layout

The app is a fixed desktop shell. A 48px top bar holds the brand, a fused repo / branch breadcrumb, a search field with its scope tabs joined to its left edge, and the actions (pin, reload, feedback, account). Under it, a four-column grid: a 232px sidebar, the centre column, a resize bar, and a 380px inspector (`--details-w`, user-resizable). At 1200px and below, the sidebar is 208px and the inspector is 340px. The inspector can be turned off, and the centre then takes its width.

The centre column has a sticky 48px toolbar (title, count, ranking, filters, active pills, cell-size toggle), the grid, and the export strip docked flush at its foot. The grid fills columns of at least 120px (small) or 180px (large), with 12px row gaps and 8px column gaps. The export strip is one 48px row at rest and grows upward when opened, to at most 32vh or 300px. Under a 760px container width, it drops the budget name and shortens the meter so both buttons stay on the row.

Settings is a full-height sheet from the right, at most 680px wide, over a 55% black scrim. Each setting is a row: a 176px heading column and a control column, with 24px vertical padding and a hairline between rows. Under 600px the row stacks.

Spacing steps are 4, 8, 12, 16, 24 and 32px (`--sp-1` to `--sp-6`). Controls are 28px (buttons, rows), 30px (fields) and 24px (small buttons, chips). Panel insets are 16px horizontally. Sidebar rows have a 2px gap, and sidebar groups have 14px between them.

### Named Rules
**The Dense Bench Rule.** Rows are 28px and cells start at 120px. Do not add height or padding to make a surface "breathe". The density is the product.

## Elevation & Depth

The shell is flat. Depth comes from three tonal steps (ground, panel, raise) and 1px hairlines. Shadows appear only on surfaces that float over the shell: menus, popovers, the search suggestions, the multi-select bar, dialogs and the Settings sheet. The stage has its own physical depth: a soft radial floor shadow under the asset, the same in a card and in a viewer.

### Shadow Vocabulary
- **Menu** (`box-shadow: 0 1px 2px rgba(0,0,0,0.4), 0 10px 28px rgba(0,0,0,0.5)`): dropdowns, pickers, popovers, context menus, the multi-select bar and the menu notice.
- **Pop** (`box-shadow: 0 12px 32px rgba(0,0,0,0.45)`): dialogs and the search suggestions.
- **Sheet** (`box-shadow: -16px 0 40px rgba(0,0,0,0.4)`): the Settings sheet edge.
- **Floor** (`radial-gradient(closest-side, rgba(0,0,0,0.4), transparent)` under a card, 0.32 under a viewer): the asset's own shadow on the stage.

### Named Rules
**The Flat Shell Rule.** Nothing in the shell casts a shadow at rest. A shadow means "this floats over the app" or "this is the asset's floor".

## Shapes

Corners are gently rounded and small: 4px for chips, inner tabs, context rows and checkboxes, 6px for buttons, fields, rows, cards and stages, and 8px for floating panels and dialogs. Pills (filter pills at 12px, badges, counts, the update pill and the viewer hint at 999px) are the only round forms. Circles are reserved for the account avatar, the brand mark and the calibration balls. Type swatches are 2px-rounded squares. They read as checker patches, not as round status dots. State dots are circles. Hover rings on cards are drawn as two stacked box-shadows: a ground gap, then a Dim Grey ring at hover or a Paper White ring at 4px when selected. They are not borders.

## Components

### Buttons
Plain, compact and confident. One weight of emphasis per surface.
- **Shape:** gently curved (6px), 28px high, 1px border.
- **Primary:** Checker Orange fill and border, Burnt Umber Ink text at weight 600. Large primary (36px, 16px padding, 13px) only on sign-in.
- **Hover / Focus:** primary lifts to the hover orange. Secondary mixes 18% Paper White into Raise and takes the strong hairline. Focus is a 2px Paper White outline at 2px offset on every control.
- **Secondary:** Raise fill, hairline border, Paper White text at 500. Disabled at 50% opacity.
- **Icon button:** a 28px square in Mute Grey that turns Paper White on hover. Pressed, it takes the Ground fill and the strong hairline. `.labeled` adds one short word.
- **Sign out:** transparent with a Calm Blue border mix and Calm Blue text.

### Chips
- **Type chip (on a card):** translucent ground (88%), 18px high, 4px radius, a 7px type-patch swatch, then the full sidebar type label in Paper White at 11px/500.
- **Filter chip:** 24px, transparent with a hairline, Soft Paper text. Pressed, it takes Raise, the strong hairline and Paper White. "More" uses a dashed hairline.
- **Active-filter pill:** 24px, 12px radius, Raise fill, with a remove mark in Mute Grey.

### Cards / Containers
- **Asset card:** no frame. The stage is the card: a square, 6px-rounded Stage Grey tile with a radial lift and the floor shadow. Under it, the name (12px/500) and a Measure line of facts joined by Faint middle dots.
- **Marks on the stage:** type chip top-left, counts and the multi-pick number top-right, "new" tags bottom-left, the favourite star bottom-right. All sit on translucent ground. A 3px budget-weight line runs along the stage foot and turns Soft Red past the limit.
- **Selection:** a Paper White ring with a ground gap. A picked card in a multi-selection also lightens its stage by 12% white.
- **Panels:** sidebar, inspector and export strip are Graphite Panel with one hairline edge, no radius and no shadow.

### Inputs / Fields
- **Style:** 30px, Ground fill, hairline border, 6px radius, Dim Grey placeholder. Selects use Raise.
- **Focus:** the border goes to the strong hairline, with a 2px Paper White outline at 1px offset. The search field takes the same ring on `:focus-within`.
- **Checkbox:** drawn, 16px, 4px radius. Checked, it fills Paper White with a ground tick. It is never orange.
- **Error:** the error note uses the danger tint background and border, with text in Paper White.

### Navigation
- **Sidebar rows:** 28px, transparent, Mute Grey at 12px. Hover takes 55% Raise and Paper White. Pressed takes full Raise and Paper White. Counts sit right-aligned in Measure. Type rows lead with an 8px type-patch square. Other rows lead with a 14px stroke icon.
- **Group heads:** a Label in Dim Grey with a chevron that rotates 90° when open.
- **Breadcrumb and scope:** one bordered control per group, its segments split by hairlines. The pressed scope tab is Raise on a Ground track.

### Calibration strip (signature)
A 34px Ground strip under the inspector viewer that states the viewing condition: an 18px grey ball, a chrome ball, a row of 16px checker patches, and the stage reading ("Stage 18% grey") in Measure at the right.

### Viewer (signature)
The inspector viewer is a Stage Grey field with the floor shadow. Its controls are a translucent set (82% ground, 12% white hairline, 24px, 11px text, 4px blur). The set appears on hover or focus and stays visible on touch. Live readouts (time, frame) are Measure on a dark pill. The 3D viewer adds a faint neutral floor grid (centre #666666, lines #6f6f6f) for spatial reference. Captured thumbnails are drawn without the grid and with a transparent background, so the page's stage shows through.

### Dependency list
Rows of 28px at 13px. A 40px column holds short type codes (MAT, TEX…) in 11px Mute Grey, because that column is too narrow for the full label. Children hang from drawn tree branches in the strong hairline.

### Export strip
Docked at the foot of the centre column, Graphite Panel with one top hairline. The row holds the toggle, the summary (assets and dependencies), the size, an optional budget meter (Paper White fill, then yellow or red), Import into Unity, and the orange Export button.

### Motion
Motion is short and functional, on `cubic-bezier(0.2, 0.8, 0.2, 1)`. There are three events. The export-list summary flashes once (0.9s) when an item is added. The Settings sheet slides in 24px from the right (0.16s). Dialogs rise 6px (0.14s). Otherwise, only state transitions of 0.1–0.15s are used: chevrons, checkbox ticks, splitter lines and hover rings. Under `prefers-reduced-motion`, all of it stops, including the effect playback on cards, spinners and shimmer.

## Do's and Don'ts

### Do:
- **Do** put every asset preview on Stage Grey (#767676) with the soft floor shadow, in cards and viewers alike.
- **Do** give each surface one orange primary action, with Burnt Umber Ink text on it.
- **Do** mark asset types only with the colour-checker patch as a small square swatch, and keep the type word in text colour.
- **Do** use the four sizes (11, 12, 13, 16px) and build hierarchy from weight and text colour.
- **Do** set sizes, counts, times, versions and paths in Geist Mono with tabular figures.
- **Do** separate shell regions with 1px hairlines and tonal steps. Keep shadows for floating panels.
- **Do** show focus as a 2px Paper White outline on every interactive element.
- **Do** keep 28px rows and 120px minimum cells.
- **Do** stop all motion under `prefers-reduced-motion`.

### Don't:
- **Don't** use orange for selection, pressed, favourite or "on" states. Use Paper White, Raise and Ground.
- **Don't** colour a surface, border or text with a type patch. The patch is a swatch, not a theme.
- **Don't** add a display size, a fifth text size or a remote font.
- **Don't** draw the 3D floor grid into a captured thumbnail.
- **Don't** default a preview to a dark or coloured backdrop. Stage Grey is the default, and a dark backdrop is the user's choice.
- **Don't** use the danger red for a normal choice such as signing out.
- **Don't** add decoration that carries no information: no gradients on the shell, no glow, no neon accent, no large showroom cards.

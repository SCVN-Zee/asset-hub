---
version: 1
slug: "ui-src-screens-browse-tsx"
primary_target: "ui/src/screens/Browse.tsx"
related_targets: ["ui/src/screens/Settings.tsx","ui/src/screens/SignIn.tsx"]
---

Scope: the whole Asset Hub app (Browse, inspector, export list, Settings, sign-in). Mode: Operate.
Audience: the whole studio; job: find a Unity asset on GitHub, judge it, export it with dependencies. Constraints: PRODUCT.md; no remote fonts; en/ko/vi.

## Direction contract

THESIS: Every asset is judged on a calibrated mid-grey stage, the way look-dev artists check their work. It refuses the dark-panel asset grid with huge cards, a neon accent and decoration that carries no information.

OWN-WORLD: Graphite shell (#1f1f20 ground, #2a2a2b panels, #e9e9e6 text). Every thumbnail and viewer sits on a true 18% grey stage (#767676) with a soft floor shadow. Asset types are coloured with colour-checker patches, on small type chips and sidebar dots only. The checker's orange (#d67e2c) marks the primary action. Geist and Geist Mono, four fixed size steps; mono only for measurements.

STORY: The visitor knows where everything is at once, scans hundreds of assets on one neutral light table, reads the facts that decide (weight, triangles, budget fit), and exports from one obvious place.

FIRST VIEWPORT: Title strip, one 48px top bar (breadcrumb repo/branch, scope fused to search, pin/reload/feedback/account), 232px sidebar (types, yours, pinned, folders), dense grid of ~126px cells, 380px inspector with viewer and calibration strip, export strip docked at the bottom of the centre with the orange Export button.

FORM: Calibration (look-dev grey ball, chrome ball, colour checker), position 1 of 7, kind pick. seed:d2ed57d2ba47321e

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

# ADR 0006: Interface palette, icons and motion

- Status: accepted (requested by the project owner)
- Date: 2026-10-13

## Context

The first theme used cadcraft's blue-gray surfaces with a blue accent and hand-drawn icons. Reviewing it, the project owner asked for neutral surfaces (Tailwind neutral 900 to 950 in the dark theme) with yellow 400 as the primary color, a more modern look than cadcraft's where that helps, and an interface that feels smooth. Buttons, the tool column, tooltips, menus, inputs and the Properties panels were called out as needing a proper design, and an icon library such as Lucide or Tabler was suggested.

## Decision

- **Palette** (`crates/ui/src/theme.rs`): dark surfaces from neutral 950 (`#0a0a0a`, borders and the canvas) through neutral 900 (`#171717`, chrome) to neutral 800 (`#262626`, controls); text neutral 200, 400 and 500. The light theme uses neutral 50 to 200 with neutral 900 text. Both themes use yellow 400 (`#facc15`) for primary fills (the active tool, the selected tab) with neutral 900 content on it; yellow lines and text on light surfaces use yellow 600 for contrast. Property cards, popups and key caps have their own tokens. A test checks text contrast (WCAG: 7 for body text, 4.5 for secondary text and for text on the accent).
- **Icons:** Tabler Icons (MIT), whose outline set is drawn with SVG paths only. The path data of the icons in use is kept as text (`icon_data.rs`), parsed once by a small SVG path parser (`vector_icon.rs`) and stroked thinner than the source, so icons stay crisp at any size and follow the theme colors. No icon font or image files are bundled.
- **Components:** a tool rail (icon pill with a small label under it, accent fill when active), segmented groups for toggles, outlined chips for actions on the selection, property cards with eased opening, tooltips with a title, a description and the shortcut as key caps, menus with icons, section headers, check marks and shortcuts, and search fields with a centered icon and placeholder.
- **Motion:** hover, selection and opening states ease over 0.08 to 0.16 seconds instead of switching at once.

## Consequences

- New widgets take their colors from the tokens and their shapes from `widgets.rs`, so both themes stay consistent.
- Adding an icon means copying its path data from Tabler (or drawing one in the same 24 unit grid) into `icon_data.rs`; the attribution is in ATTRIBUTION.md and NOTICE.

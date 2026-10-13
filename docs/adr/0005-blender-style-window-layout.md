# ADR 0005: Blender-style window layout

- Status: accepted (requested by the project owner)
- Date: 2026-10-13

## Context

The first GUI shell followed the cadcraft layout: a title row, one toolbar row holding every button (file, history, transform tools, selection modes, primitives, view toggles, shading), a dockable Tool Sets panel on the left, and an in-window menu bar only off macOS. Reviewing it, the project owner asked for a layout people know from Blender: tools as a vertical column of icon buttons on the left, actions on the selection on top, menus always visible, a clickable navigation gizmo in the top right corner of the viewport, and a Blender-like Outliner and Properties editor on the right.

## Decision

The window is laid out as follows (project spec 20.4 is updated to match):

- **Top bar:** the menus (also the native macOS menu bar), the document title, and the workspace tabs on the right.
- **Tool settings row:** file and history buttons, the active tool, and, while objects are selected, the actions on them (the contextual toolbar).
- **Tool column (left):** a narrow column of icon buttons in groups (transform tools, then creation); labels on request. It is window chrome, not a dock panel.
- **Viewport:** a header strip with selection modes, view toggles and shading; the navigation gizmo in the top right corner (axis balls: click to view from an axis, drag to orbit; round zoom, pan, projection and frame buttons below).
- **Right column:** Outliner (search, tree, viewport-only eye toggles) above Properties (a tab column with collapsible panels; values edited through ops with schema-generated forms), then Health.
- **Bottom:** History and Console tabs, which can be hidden.
- **Status bar:** mode, selection, last message, triangles against the budget, units, snapping, asset class and profile, jobs, frame rate.

The view cube of spec 20.5 is replaced by the navigation gizmo.

## Consequences

- Panels that arrive later (UV Editor, Node Graph, Timeline, Assistant, Bake, Export) join the bottom tabs or the right column; their workspaces become available as they land.
- Tools and actions stay commands, so menus, shortcuts, the command palette, the control server and agents keep sharing one path.

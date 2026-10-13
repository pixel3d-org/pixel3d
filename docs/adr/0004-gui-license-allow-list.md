# ADR 0004: License allow list for the GUI stack

- Status: accepted (approved by the project owner)
- Date: 2026-10-13

## Context

The desktop app is built on eframe, as the GUI stack requires. Two of its dependencies carry licenses outside the allow list in `deny.toml`:

1. eframe always enables egui-winit's clipboard support, which uses arboard. On Windows, arboard depends on `clipboard-win` and `error-code`, both under the Boost Software License 1.0 (BSL-1.0). There is no feature to turn the clipboard off.
2. eframe's `default_fonts` feature compiles egui's bundled fonts (`epaint_default_fonts`): Ubuntu-Light under the Ubuntu Font Licence 1.0, Noto Emoji under the SIL Open Font License 1.1, Hack under the MIT and Bitstream Vera licenses, and an MIT icon font.

BSL-1.0 is a short permissive license (OSI approved) that requires no notice in binary distributions. OFL-1.1 and the Ubuntu Font Licence allow embedding and redistributing the fonts in software, with their license texts.

The alternatives were to replace eframe with a custom window host (a large amount of work for no product benefit) and to load the operating system's fonts instead of bundled ones (font coverage then differs per machine).

## Decision

The project owner approved both on 2026-10-13:

- `BSL-1.0` joins the allow list in `deny.toml`.
- `OFL-1.1` and `Ubuntu-font-1.0` are allowed only for the `epaint_default_fonts` crate, as a cargo-deny exception. Any other crate under these licenses is still rejected.
- eframe keeps its `default_fonts` feature.

## Consequences

- NOTICE and ATTRIBUTION.md list the crates, fonts and licenses.
- Installers (Phase 17) ship the font license texts next to the license files of pixel3d.
- A new dependency under any other license outside the allow list still needs an explicit decision and an ADR.

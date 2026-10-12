# Asset attribution

Every non-code asset in this repository (images, icons, fonts, sample models, textures, presets) is listed here with its author, source and license.

**Policy:** only original work by pixel3d contributors, or third-party material under a license that allows redistribution (OSI open source, public domain or CC0, or a Creative Commons license that allows redistribution). Material from GPL projects is never copied. Each third-party sample model (for example from the Khronos glTF sample models) is checked individually before it is added.

Generated-in-code assets are original and have no file to list: the interface icons are painted in code (`crates/ui/src/icons.rs`, in the style of cadcraft's icon set, see NOTICE).

Fonts compiled into the desktop app come from egui's `epaint_default_fonts` crate (version 0.36.2); none are stored in this repository (ADR 0004).

| Asset | Author | Source | License | Notes |
|---|---|---|---|---|
| Ubuntu Light font | Dalton Maag Ltd for Canonical Ltd | `epaint_default_fonts` 0.36.2 (https://github.com/emilk/egui) | Ubuntu Font Licence 1.0 | Interface text in the desktop app |
| Noto Emoji font | Google Inc. | `epaint_default_fonts` 0.36.2 | SIL Open Font License 1.1 | Symbol fallback in the desktop app |
| Hack font | Source Foundry Authors | `epaint_default_fonts` 0.36.2 | MIT and Bitstream Vera License | Monospace text in the desktop app |
| emoji-icon-font | John Slegers | `epaint_default_fonts` 0.36.2 | MIT | Icon glyph fallback in the desktop app |

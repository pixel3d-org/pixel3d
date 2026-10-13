//! Icon outlines as SVG path data on a 24 x 24 grid. Most come from Tabler
//! Icons 3.34.1 (MIT, https://tabler.io/icons, see ATTRIBUTION.md); the
//! constants after `// Drawn for pixel3d` follow the same grid and style.

pub const FILE_PLUS: &[&str] =
    &["M14 3v4a1 1 0 0 0 1 1h4", "M17 21h-10a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2h7l5 5v11a2 2 0 0 1 -2 2z", "M12 11l0 6", "M9 14l6 0"];
pub const FOLDER_OPEN: &[&str] = &[
    "M5 19l2.757 -7.351a1 1 0 0 1 .936 -.649h12.307a1 1 0 0 1 .986 1.164l-.996 5.211a2 2 0 0 1 -1.964 1.625h-14.026a2 2 0 0 1 -2 -2v-11a2 2 0 0 1 2 -2h4l3 3h7a2 2 0 0 1 2 2v2",
];
pub const DEVICE_FLOPPY: &[&str] =
    &["M6 4h10l4 4v10a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2v-12a2 2 0 0 1 2 -2", "M12 14m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0", "M14 4l0 4l-6 0l0 -4"];
pub const ARROW_BACK_UP: &[&str] = &["M9 14l-4 -4l4 -4", "M5 10h11a4 4 0 1 1 0 8h-1"];
pub const ARROW_FORWARD_UP: &[&str] = &["M15 14l4 -4l-4 -4", "M19 10h-11a4 4 0 1 0 0 8h1"];
pub const POINTER: &[&str] = &[
    "M7.904 17.563a1.2 1.2 0 0 0 2.228 .308l2.09 -3.093l4.907 4.907a1.067 1.067 0 0 0 1.509 0l1.047 -1.047a1.067 1.067 0 0 0 0 -1.509l-4.907 -4.907l3.113 -2.09a1.2 1.2 0 0 0 -.309 -2.228l-13.582 -3.904l3.904 13.563z",
];
pub const ARROWS_MOVE: &[&str] =
    &["M18 9l3 3l-3 3", "M15 12h6", "M6 9l-3 3l3 3", "M3 12h6", "M9 18l3 3l3 -3", "M12 15v6", "M15 6l-3 -3l-3 3", "M12 3v6"];
pub const ROTATE_3D: &[&str] =
    &["M12 3a7 7 0 0 1 7 7v4l-3 -3", "M22 11l-3 3", "M8 15.5l-5 -3l5 -3l5 3v5.5l-5 3z", "M3 12.5v5.5l5 3", "M8 15.545l5 -3.03"];
pub const RESIZE: &[&str] =
    &["M4 11v8a1 1 0 0 0 1 1h8m-9 -14v-1a1 1 0 0 1 1 -1h1m5 0h2m5 0h1a1 1 0 0 1 1 1v1m0 5v2m0 5v1a1 1 0 0 1 -1 1h-1", "M4 12h7a1 1 0 0 1 1 1v7"];
pub const CUBE: &[&str] = &[
    "M21 16.008v-8.018a1.98 1.98 0 0 0 -1 -1.717l-7 -4.008a2.016 2.016 0 0 0 -2 0l-7 4.008c-.619 .355 -1 1.01 -1 1.718v8.018c0 .709 .381 1.363 1 1.717l7 4.008a2.016 2.016 0 0 0 2 0l7 -4.008c.619 -.355 1 -1.01 1 -1.718z",
    "M12 22v-10",
    "M12 12l8.73 -5.04",
    "M3.27 6.96l8.73 5.04",
];
pub const CYLINDER: &[&str] = &["M12 6m-7 0a7 3 0 1 0 14 0a7 3 0 1 0 -14 0", "M5 6v12c0 1.657 3.134 3 7 3s7 -1.343 7 -3v-12"];
pub const SPHERE: &[&str] = &["M3 12c0 1.657 4.03 3 9 3s9 -1.343 9 -3", "M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0"];
pub const ASTERISK: &[&str] = &["M12 12l8 -4.5", "M12 12v9", "M12 12l-8 -4.5", "M12 12l8 4.5", "M12 3v9", "M12 12l-8 4.5"];
pub const FOCUS_CENTERED: &[&str] = &[
    "M12 12m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
    "M4 8v-2a2 2 0 0 1 2 -2h2",
    "M4 16v2a2 2 0 0 0 2 2h2",
    "M16 4h2a2 2 0 0 1 2 2v2",
    "M16 20h2a2 2 0 0 0 2 -2v-2",
];
pub const FILLED_CIRCLE: &[&str] = &["M7 3.34a10 10 0 1 1 -4.995 8.984l-.005 -.324l.005 -.324a10 10 0 0 1 4.995 -8.336z"];
pub const WORLD: &[&str] =
    &["M3 12a9 9 0 1 0 18 0a9 9 0 0 0 -18 0", "M3.6 9h16.8", "M3.6 15h16.8", "M11.5 3a17 17 0 0 0 0 18", "M12.5 3a17 17 0 0 1 0 18"];
pub const INNER_SHADOW_BOTTOM_RIGHT: &[&str] = &["M12 21a9 9 0 1 1 0 -18a9 9 0 0 1 0 18z", "M18 12a6 6 0 0 1 -6 6"];
pub const LAYERS_INTERSECT: &[&str] = &[
    "M8 4m0 2a2 2 0 0 1 2 -2h8a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-8a2 2 0 0 1 -2 -2z",
    "M4 8m0 2a2 2 0 0 1 2 -2h8a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-8a2 2 0 0 1 -2 -2z",
];
pub const GRID_4X4: &[&str] = &["M3 6h18", "M3 12h18", "M3 18h18", "M6 3v18", "M12 3v18", "M18 3v18"];
pub const PERSPECTIVE: &[&str] =
    &["M6.141 4.163l12 1.714a1 1 0 0 1 .859 .99v10.266a1 1 0 0 1 -.859 .99l-12 1.714a1 1 0 0 1 -1.141 -.99v-13.694a1 1 0 0 1 1.141 -.99z"];
pub const BOX_MODEL_2: &[&str] = &["M8 8h8v8h-8z", "M4 4m0 2a2 2 0 0 1 2 -2h12a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2z"];
pub const SEARCH: &[&str] = &["M10 10m-7 0a7 7 0 1 0 14 0a7 7 0 1 0 -14 0", "M21 21l-6 -6"];
pub const SETTINGS: &[&str] = &[
    "M10.325 4.317c.426 -1.756 2.924 -1.756 3.35 0a1.724 1.724 0 0 0 2.573 1.066c1.543 -.94 3.31 .826 2.37 2.37a1.724 1.724 0 0 0 1.065 2.572c1.756 .426 1.756 2.924 0 3.35a1.724 1.724 0 0 0 -1.066 2.573c.94 1.543 -.826 3.31 -2.37 2.37a1.724 1.724 0 0 0 -2.572 1.065c-.426 1.756 -2.924 1.756 -3.35 0a1.724 1.724 0 0 0 -2.573 -1.066c-1.543 .94 -3.31 -.826 -2.37 -2.37a1.724 1.724 0 0 0 -1.065 -2.572c-1.756 -.426 -1.756 -2.924 0 -3.35a1.724 1.724 0 0 0 1.066 -2.573c-.94 -1.543 .826 -3.31 2.37 -2.37c1 .608 2.296 .07 2.572 -1.065z",
    "M9 12a3 3 0 1 0 6 0a3 3 0 0 0 -6 0",
];
pub const HELP: &[&str] = &["M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0", "M12 17l0 .01", "M12 13.5a1.5 1.5 0 0 1 1 -1.5a2.6 2.6 0 1 0 -3 -4"];
pub const PLUS: &[&str] = &["M12 5l0 14", "M5 12l14 0"];
pub const X: &[&str] = &["M18 6l-12 12", "M6 6l12 12"];
pub const CHEVRON_DOWN: &[&str] = &["M6 9l6 6l6 -6"];
pub const CHEVRON_RIGHT: &[&str] = &["M9 6l6 6l-6 6"];
pub const MENU_2: &[&str] = &["M4 6l16 0", "M4 12l16 0", "M4 18l16 0"];
pub const EYE: &[&str] =
    &["M10 12a2 2 0 1 0 4 0a2 2 0 0 0 -4 0", "M21 12c-2.4 4 -5.4 6 -9 6c-3.6 0 -6.6 -2 -9 -6c2.4 -4 5.4 -6 9 -6c3.6 0 6.6 2 9 6"];
pub const EYE_OFF: &[&str] = &[
    "M10.585 10.587a2 2 0 0 0 2.829 2.828",
    "M16.681 16.673a8.717 8.717 0 0 1 -4.681 1.327c-3.6 0 -6.6 -2 -9 -6c1.272 -2.12 2.712 -3.678 4.32 -4.674m2.86 -1.146a9.055 9.055 0 0 1 1.82 -.18c3.6 0 6.6 2 9 6c-.666 1.11 -1.379 2.067 -2.138 2.87",
    "M3 3l18 18",
];
pub const LOCK: &[&str] = &[
    "M5 13a2 2 0 0 1 2 -2h10a2 2 0 0 1 2 2v6a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2v-6z",
    "M11 16a1 1 0 1 0 2 0a1 1 0 0 0 -2 0",
    "M8 11v-4a4 4 0 1 1 8 0v4",
];
pub const LOCK_OPEN: &[&str] = &[
    "M5 11m0 2a2 2 0 0 1 2 -2h10a2 2 0 0 1 2 2v6a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2z",
    "M12 16m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
    "M8 11v-5a4 4 0 0 1 8 0",
];
pub const VECTOR_TRIANGLE: &[&str] = &[
    "M10 4m0 1a1 1 0 0 1 1 -1h2a1 1 0 0 1 1 1v2a1 1 0 0 1 -1 1h-2a1 1 0 0 1 -1 -1z",
    "M3 17m0 1a1 1 0 0 1 1 -1h2a1 1 0 0 1 1 1v2a1 1 0 0 1 -1 1h-2a1 1 0 0 1 -1 -1z",
    "M17 17m0 1a1 1 0 0 1 1 -1h2a1 1 0 0 1 1 1v2a1 1 0 0 1 -1 1h-2a1 1 0 0 1 -1 -1z",
    "M6.5 17.1l5 -9.1",
    "M17.5 17.1l-5 -9.1",
    "M7 19l10 0",
];
pub const ACTIVITY_HEARTBEAT: &[&str] = &["M3 12h4.5l1.5 -6l4 12l2 -9l1.5 3h4.5"];
pub const HISTORY: &[&str] = &["M12 8l0 4l2 2", "M3.05 11a9 9 0 1 1 .5 4m-.5 5v-5h5"];
pub const LIST_TREE: &[&str] = &["M9 6h11", "M12 12h8", "M15 18h5", "M5 6v.01", "M8 12v.01", "M11 18v.01"];
pub const ADJUSTMENTS_HORIZONTAL: &[&str] = &[
    "M14 6m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0",
    "M4 6l8 0",
    "M16 6l4 0",
    "M8 12m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0",
    "M4 12l2 0",
    "M10 12l10 0",
    "M17 18m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0",
    "M4 18l11 0",
    "M19 18l1 0",
];
pub const TERMINAL_2: &[&str] = &["M8 9l3 3l-3 3", "M13 15l3 0", "M3 4m0 2a2 2 0 0 1 2 -2h14a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2z"];
pub const TOOL: &[&str] = &["M7 10h3v-3l-3.5 -3.5a6 6 0 0 1 8 8l6 6a2 2 0 0 1 -3 3l-6 -6a6 6 0 0 1 -8 -8l3.5 3.5"];
pub const STACK_2: &[&str] = &["M12 4l-8 4l8 4l8 -4l-8 -4", "M4 12l8 4l8 -4", "M4 16l8 4l8 -4"];
pub const COPY: &[&str] = &[
    "M7 7m0 2.667a2.667 2.667 0 0 1 2.667 -2.667h8.666a2.667 2.667 0 0 1 2.667 2.667v8.666a2.667 2.667 0 0 1 -2.667 2.667h-8.666a2.667 2.667 0 0 1 -2.667 -2.667z",
    "M4.012 16.737a2.005 2.005 0 0 1 -1.012 -1.737v-10c0 -1.1 .9 -2 2 -2h10c.75 0 1.158 .385 1.5 1",
];
pub const TRASH: &[&str] =
    &["M4 7l16 0", "M10 11l0 6", "M14 11l0 6", "M5 7l1 12a2 2 0 0 0 2 2h8a2 2 0 0 0 2 -2l1 -12", "M9 7v-3a1 1 0 0 1 1 -1h4a1 1 0 0 1 1 1v3"];
pub const CHECKS: &[&str] = &["M7 12l5 5l10 -10", "M2 12l5 5m5 -5l5 -5"];
pub const BUCKET_DROPLET: &[&str] = &[
    "M5 16l1.465 1.638a2 2 0 1 1 -3.015 .099l1.55 -1.737z",
    "M13.737 9.737c2.299 -2.3 3.23 -5.095 2.081 -6.245c-1.15 -1.15 -3.945 -.217 -6.244 2.082c-2.3 2.299 -3.231 5.095 -2.082 6.244c1.15 1.15 3.946 .218 6.245 -2.081z",
    "M7.492 11.818c.362 .362 .768 .676 1.208 .934l6.895 4.047c1.078 .557 2.255 -.075 3.692 -1.512c1.437 -1.437 2.07 -2.614 1.512 -3.692c-.372 -.718 -1.72 -3.017 -4.047 -6.895a6.015 6.015 0 0 0 -.934 -1.208",
];
pub const ARROWS_MAXIMIZE: &[&str] =
    &["M16 4l4 0l0 4", "M14 10l6 -6", "M8 20l-4 0l0 -4", "M4 20l6 -6", "M16 20l4 0l0 -4", "M14 14l6 6", "M8 4l-4 0l0 4", "M4 4l6 6"];
pub const INFO_CIRCLE: &[&str] = &["M3 12a9 9 0 1 0 18 0a9 9 0 0 0 -18 0", "M12 9h.01", "M11 12h1v4h1"];
pub const ALERT_TRIANGLE: &[&str] = &[
    "M12 9v4",
    "M10.363 3.591l-8.106 13.534a1.914 1.914 0 0 0 1.636 2.871h16.214a1.914 1.914 0 0 0 1.636 -2.87l-8.106 -13.536a1.914 1.914 0 0 0 -3.274 0z",
    "M12 16h.01",
];
pub const KEYBOARD: &[&str] = &[
    "M2 6m0 2a2 2 0 0 1 2 -2h16a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2z",
    "M6 10l0 .01",
    "M10 10l0 .01",
    "M14 10l0 .01",
    "M18 10l0 .01",
    "M6 14l0 .01",
    "M18 14l0 .01",
    "M10 14l4 .01",
];

pub const FOLDER: &[&str] = &["M5 4h4l3 3h7a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-11a2 2 0 0 1 2 -2"];
pub const BOX_MULTIPLE: &[&str] = &[
    "M7 3m0 2a2 2 0 0 1 2 -2h10a2 2 0 0 1 2 2v10a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2z",
    "M17 17v2a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2v-10a2 2 0 0 1 2 -2h2",
];
pub const TRIANGLE: &[&str] =
    &["M10.363 3.591l-8.106 13.534a1.914 1.914 0 0 0 1.636 2.871h16.214a1.914 1.914 0 0 0 1.636 -2.87l-8.106 -13.536a1.914 1.914 0 0 0 -3.274 0z"];
pub const STACK_3: &[&str] = &["M12 2l-8 4l8 4l8 -4l-8 -4", "M4 10l8 4l8 -4", "M4 18l8 4l8 -4", "M4 14l8 4l8 -4"];
pub const ARROW_BAR_UP: &[&str] = &["M12 4l0 10", "M12 4l4 4", "M12 4l-4 4", "M4 20l16 0"];
pub const BORDER_INNER: &[&str] = &[
    "M4 12l16 0",
    "M12 4l0 16",
    "M4 4l0 .01",
    "M8 4l0 .01",
    "M16 4l0 .01",
    "M20 4l0 .01",
    "M4 8l0 .01",
    "M20 8l0 .01",
    "M4 16l0 .01",
    "M20 16l0 .01",
    "M4 20l0 .01",
    "M8 20l0 .01",
    "M16 20l0 .01",
    "M20 20l0 .01",
];
pub const BORDER_CORNERS: &[&str] =
    &["M16 4h2a2 2 0 0 1 2 2v2", "M20 16v2a2 2 0 0 1 -2 2h-2", "M8 20h-2a2 2 0 0 1 -2 -2v-2", "M4 8v-2a2 2 0 0 1 2 -2h2"];
pub const LAYOUT_COLUMNS: &[&str] = &["M4 4m0 2a2 2 0 0 1 2 -2h12a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2z", "M12 4l0 16"];
pub const WAVE_SINE: &[&str] = &[
    "M21 12h-2c-.894 0 -1.662 -.857 -1.761 -2c-.296 -3.45 -.749 -6 -2.749 -6s-2.5 3.582 -2.5 8s-.5 8 -2.5 8s-2.452 -2.547 -2.749 -6c-.1 -1.147 -.867 -2 -1.763 -2h-2",
];
pub const ARROWS_HORIZONTAL: &[&str] = &["M7 8l-4 4l4 4", "M17 8l4 4l-4 4", "M3 12l18 0"];
pub const ARROW_AUTOFIT_HEIGHT: &[&str] =
    &["M12 20h-6a2 2 0 0 1 -2 -2v-12a2 2 0 0 1 2 -2h6", "M18 14v7", "M18 3v7", "M15 18l3 3l3 -3", "M15 6l3 -3l3 3"];
pub const SCISSORS: &[&str] =
    &["M6 7m-3 0a3 3 0 1 0 6 0a3 3 0 1 0 -6 0", "M6 17m-3 0a3 3 0 1 0 6 0a3 3 0 1 0 -6 0", "M8.6 8.6l10.4 10.4", "M8.6 15.4l10.4 -10.4"];
pub const SQUARE_DOT: &[&str] =
    &["M3 3m0 2a2 2 0 0 1 2 -2h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2z", "M12 12m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0"];
pub const TRIANGLES: &[&str] = &[
    "M9.974 21h8.052a.975 .975 0 0 0 .81 -1.517l-4.025 -6.048a.973 .973 0 0 0 -1.622 0l-4.025 6.048a.977 .977 0 0 0 .81 1.517z",
    "M4.98 16h14.04c.542 0 .98 -.443 .98 -.989a1 1 0 0 0 -.156 -.534l-7.02 -11.023a.974 .974 0 0 0 -1.648 0l-7.02 11.023a1 1 0 0 0 .294 1.366a.973 .973 0 0 0 .53 .157z",
];
pub const ARROWS_JOIN: &[&str] = &["M3 7h5l3.5 5h9.5", "M3 17h5l3.495 -5", "M18 15l3 -3l-3 -3"];
pub const ERASER: &[&str] =
    &["M19 20h-10.5l-4.21 -4.3a1 1 0 0 1 0 -1.41l10 -10a1 1 0 0 1 1.41 0l5 5a1 1 0 0 1 0 1.41l-9.2 9.3", "M18 13.3l-6.3 -6.3"];
pub const GRID_3X3: &[&str] = &["M3 8h18", "M3 16h18", "M8 3v18", "M16 3v18"];
pub const BUILDING_BRIDGE: &[&str] =
    &["M6 5l0 14", "M18 5l0 14", "M2 15l20 0", "M3 8a7.5 7.5 0 0 0 3 -2a6.5 6.5 0 0 0 12 0a7.5 7.5 0 0 0 3 2", "M12 10l0 5"];
pub const ARROWS_SPLIT: &[&str] = &["M21 17h-8l-3.5 -5h-6.5", "M21 7h-8l-3.495 5", "M18 10l3 -3l-3 -3", "M18 20l3 -3l-3 -3"];
pub const FLIP_VERTICAL: &[&str] = &["M12 3l0 18", "M16 7l0 10l5 0l-5 -10", "M8 7l0 10l-5 0l5 -10"];
pub const COMPASS: &[&str] =
    &["M8 16l2 -6l6 -2l-2 6l-6 2", "M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0", "M12 3l0 2", "M12 19l0 2", "M3 12l2 0", "M19 12l2 0"];
pub const FOCUS_2: &[&str] = &["M12 12m-7 0a7 7 0 1 0 14 0a7 7 0 1 0 -14 0", "M12 3l0 2", "M3 12l2 0", "M12 19l0 2", "M19 12l2 0"];
pub const MAGNET: &[&str] =
    &["M4 13v-8a2 2 0 0 1 2 -2h1a2 2 0 0 1 2 2v8a2 2 0 0 0 6 0v-8a2 2 0 0 1 2 -2h1a2 2 0 0 1 2 2v8a8 8 0 0 1 -16 0", "M4 8l5 0", "M15 8l4 0"];
pub const CIRCLE_DASHED: &[&str] = &[
    "M8.56 3.69a9 9 0 0 0 -2.92 1.95",
    "M3.69 8.56a9 9 0 0 0 -.69 3.44",
    "M3.69 15.44a9 9 0 0 0 1.95 2.92",
    "M8.56 20.31a9 9 0 0 0 3.44 .69",
    "M15.44 20.31a9 9 0 0 0 2.92 -1.95",
    "M20.31 15.44a9 9 0 0 0 .69 -3.44",
    "M20.31 8.56a9 9 0 0 0 -1.95 -2.92",
    "M15.44 3.69a9 9 0 0 0 -3.44 -.69",
];
pub const CROSSHAIR: &[&str] =
    &["M4 8v-2a2 2 0 0 1 2 -2h2", "M4 16v2a2 2 0 0 0 2 2h2", "M16 4h2a2 2 0 0 1 2 2v2", "M16 20h2a2 2 0 0 0 2 -2v-2", "M9 12l6 0", "M12 9l0 6"];
pub const CHEVRON_UP: &[&str] = &["M6 15l6 -6l6 6"];
pub const CHECK: &[&str] = &["M5 12l5 5l10 -10"];
pub const CONE: &[&str] = &["M21 17.998v-.5l-8.13 -14.99a1 1 0 0 0 -1.74 0l-8.13 14.989v.5c0 1.659 4.03 3.003 9 3.003s9 -1.344 9 -3.002"];
pub const CAPSULE: &[&str] = &["M6 3m0 6a6 6 0 0 1 6 -6h0a6 6 0 0 1 6 6v6a6 6 0 0 1 -6 6h0a6 6 0 0 1 -6 -6z"];
pub const HEXAGON: &[&str] = &[
    "M19.875 6.27a2.225 2.225 0 0 1 1.125 1.948v7.284c0 .809 -.443 1.555 -1.158 1.948l-6.75 4.27a2.269 2.269 0 0 1 -2.184 0l-6.75 -4.27a2.225 2.225 0 0 1 -1.158 -1.948v-7.285c0 -.809 .443 -1.554 1.158 -1.947l6.75 -3.98a2.33 2.33 0 0 1 2.25 0l6.75 3.98h-.033z",
];
pub const MARQUEE_2: &[&str] =
    &["M4 6v-1a1 1 0 0 1 1 -1h1m5 0h2m5 0h1a1 1 0 0 1 1 1v1m0 5v2m0 5v1a1 1 0 0 1 -1 1h-1m-5 0h-2m-5 0h-1a1 1 0 0 1 -1 -1v-1m0 -5v-2"];
pub const CIRCLE_DOT: &[&str] = &["M12 12m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0", "M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0"];

pub const VECTOR_SPLINE: &[&str] = &[
    "M17 3m0 1a1 1 0 0 1 1 -1h2a1 1 0 0 1 1 1v2a1 1 0 0 1 -1 1h-2a1 1 0 0 1 -1 -1z",
    "M3 17m0 1a1 1 0 0 1 1 -1h2a1 1 0 0 1 1 1v2a1 1 0 0 1 -1 1h-2a1 1 0 0 1 -1 -1z",
    "M17 5c-6.627 0 -12 5.373 -12 12",
];

// Drawn for pixel3d.

/// A flat plane in perspective.
pub const PLANE: &[&str] = &["M7 7h14l-4 10h-14z", "M10 12h8"];

/// A kitchen knife: the handle low left, the blade up to the right.
pub const KNIFE: &[&str] = &["M4 20l5 -5", "M9 15l9.5 -9.5c1 -1 2.5 -.5 2 1c-1.5 4 -4.5 7.5 -9 9.5z"];

/// A torus seen from above at an angle.
pub const TORUS: &[&str] = &["M12 12m-9 0a9 5 0 1 0 18 0a9 5 0 1 0 -18 0", "M8 11.5c1 1.3 7 1.3 8 0", "M9 12.5c1 -.8 5 -.8 6 0"];

/// A wedge (ramp): a triangular prism.
pub const WEDGE: &[&str] = &["M3 18l8 3l10 -4l-8 -12z", "M11 21l2 -16"];

/// A cross section (small square) swept along a curve.
pub const SWEEP: &[&str] = &["M5 17c5 0 5 -11 15 -11", "M3 15h4v4h-4z"];

/// A pipe bending around a corner.
pub const PIPE: &[&str] = &["M4 20v-8a6 6 0 0 1 6 -6h10", "M8 20v-8a2 2 0 0 1 2 -2h10", "M4 20h4", "M20 6v4"];

/// A square section below blending into a round one above.
pub const LOFT: &[&str] = &["M4 17l8 3l8 -3l-8 -3z", "M12 6m-5 0a5 2 0 1 0 10 0a5 2 0 1 0 -10 0", "M4 17l3 -11", "M20 17l-3 -11"];

/// Three copies in a row.
pub const ARRAY: &[&str] = &["M2 10h5v5h-5z", "M9.5 10h5v5h-5z", "M17 10h5v5h-5z"];

/// Dots spread loosely over an area.
pub const SCATTER: &[&str] = &[
    "M5 6m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
    "M12 4m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
    "M19 7m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
    "M8 12m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
    "M16 13m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
    "M5 19m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
    "M12 18m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
    "M19 19m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
];

/// A page with an arrow coming in from the left.
pub const IMPORT: &[&str] = &["M14 3v4a1 1 0 0 0 1 1h4", "M5 11v-6a2 2 0 0 1 2 -2h7l5 5v11a2 2 0 0 1 -2 2h-6", "M3 16h8", "M8 13l3 3l-3 3"];

/// Two overlapping squares merged into one outline.
pub const BOOL_UNION: &[&str] = &["M4 4h11v5h5v11h-11v-5h-5z", "M9 15h3", "M15 9v3"];

/// The first square with the second cut out of it (hatched: what stays).
pub const BOOL_DIFFERENCE: &[&str] = &["M4 4h11v5h-6v6h-5z", "M15 9h5v11h-11v-5", "M4 8l4 -4", "M4 12l8 -8", "M5 15l4 -4", "M11 9l4 -4"];

/// Two overlapping squares with their shared part hatched (what stays).
pub const BOOL_INTERSECT: &[&str] = &["M4 4h11v11h-11z", "M9 9h11v11h-11z", "M9 12l3 -3", "M9 15l6 -6", "M12 15l3 -3"];

/// A large and a small sparkle: the mesh made clean.
pub const SPARKLES: &[&str] =
    &["M10 3l1.6 4.4l4.4 1.6l-4.4 1.6l-1.6 4.4l-1.6 -4.4l-4.4 -1.6l4.4 -1.6z", "M17 13l.9 2.1l2.1 .9l-2.1 .9l-.9 2.1l-.9 -2.1l-2.1 -.9l2.1 -.9z"];

/// A clipboard with lines: a report.
pub const REPORT: &[&str] =
    &["M9 5h-2a2 2 0 0 0 -2 2v12a2 2 0 0 0 2 2h10a2 2 0 0 0 2 -2v-12a2 2 0 0 0 -2 -2h-2", "M9 3h6v4h-6z", "M9 12h6", "M9 16h4"];

/// Two blobs blending into one: shapes made from blended primitives.
pub const BLOBS: &[&str] = &["M9 14m-5 0a5 5 0 1 0 10 0a5 5 0 1 0 -10 0", "M15.5 9.5m-4.5 0a4.5 4.5 0 1 0 9 0a4.5 4.5 0 1 0 -9 0"];

/// A vertex where five edges meet: a pole.
pub const POLES: &[&str] =
    &["M12 12l0 -8", "M12 12l7.6 -2.5", "M12 12l4.7 6.5", "M12 12l-4.7 6.5", "M12 12l-7.6 -2.5", "M12 12m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0"];

/// A page with code braces: a Model Spec's source.
pub const SPEC: &[&str] =
    &["M14 3v4a1 1 0 0 0 1 1h4", "M17 21h-10a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2h7l5 5v11a2 2 0 0 1 -2 2z", "M10 13l-1 2l1 2", "M14 13l1 2l-1 2"];

/// A map pin: a landmark on a surface.
pub const MAP_PIN: &[&str] =
    &["M9 11a3 3 0 1 0 6 0a3 3 0 0 0 -6 0", "M17.657 16.657l-4.243 4.243a2 2 0 0 1 -2.827 0l-4.244 -4.243a8 8 0 1 1 11.314 0z"];

/// A curve between two points with a handle: an outline drawn with Bézier
/// curves.
pub const BEZIER: &[&str] =
    &["M5 19c2 -8 6 -12 14 -14", "M3 17h4v4h-4z", "M17 3h4v4h-4z", "M5 19l7 -10", "M12 9m-1.5 0a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0"];

/// A leaf: a curved sheet from one outline.
pub const LEAF: &[&str] = &["M5 20c0 -8 4 -14 14 -15c1 10 -5 14 -14 15z", "M5 20l8 -9"];

/// A front outline beside a side outline: a blueprint.
pub const BLUEPRINT: &[&str] = &["M3 12c0 -4 3 -7 7 -7v14c-4 0 -7 -3 -7 -7z", "M14 5h4v14h-4z", "M20 5v14"];

/// A surface pulled up at a point: a deform handle.
pub const HANDLES: &[&str] = &["M3 18c4 0 5 -8 9 -8s5 8 9 8", "M12 10v-6", "M10 6l2 -2l2 2", "M12 10m-1.5 0a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0"];

/// A puzzle piece: a feature template fitted onto a model.
pub const PUZZLE: &[&str] = &[
    "M4 7h3a1 1 0 0 0 1 -1v-1a2 2 0 0 1 4 0v1a1 1 0 0 0 1 1h3a1 1 0 0 1 1 1v3a1 1 0 0 0 1 1h1a2 2 0 0 1 0 4h-1a1 1 0 0 0 -1 1v3a1 1 0 0 1 -1 1h-3a1 1 0 0 1 -1 -1v-1a2 2 0 0 0 -4 0v1a1 1 0 0 1 -1 1h-3a1 1 0 0 1 -1 -1v-3a1 1 0 0 1 1 -1h1a2 2 0 0 0 0 -4h-1a1 1 0 0 1 -1 -1v-3a1 1 0 0 1 1 -1",
];

/// Three curved strands from a root line: hair, fur, grass.
pub const STRANDS: &[&str] = &["M4 20c1 -6 -1 -11 2 -16", "M10 20c0 -6 3 -10 1 -16", "M16 20c1 -5 4 -8 3 -15", "M3 20h18"];

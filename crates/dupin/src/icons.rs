//! The settings window's icons: line drawings on a 24-point grid, stroked like OpenCode's.

use rune::Icon;

macro_rules! line_icon {
    ($name:ident, $id:literal, $paths:literal) => {
        pub static $name: Icon = Icon {
            name: concat!("dupin-", $id),
            view: 24.0,
            body: concat!(
                r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">"#,
                $paths,
                "</g>"
            ),
        };
    };
}

// Pages.
line_icon!(
    SLIDERS,
    "sliders",
    r#"<path d="M4 6h9M17 6h3M4 12h3M11 12h9M4 18h11M19 18h1"/><circle cx="15" cy="6" r="2"/><circle cx="9" cy="12" r="2"/><circle cx="17" cy="18" r="2"/>"#
);
line_icon!(
    SEARCH,
    "search",
    r#"<circle cx="11" cy="11" r="6.5"/><path d="M20 20l-4.2-4.2"/>"#
);
line_icon!(
    SPARKLES,
    "sparkles",
    r#"<path d="M10 3.5l1.6 4.4a2 2 0 0 0 1.2 1.2l4.4 1.6-4.4 1.6a2 2 0 0 0-1.2 1.2L10 17.9l-1.6-4.4a2 2 0 0 0-1.2-1.2L2.8 10.7l4.4-1.6a2 2 0 0 0 1.2-1.2z"/><path d="M18.5 15v5M16 17.5h5"/>"#
);
line_icon!(
    SHIELD,
    "shield",
    r#"<path d="M12 3l7 3v5.5c0 4.3-2.9 7.8-7 9.5-4.1-1.7-7-5.2-7-9.5V6z"/><path d="M9 12l2 2 4-4"/>"#
);

// Rows.
line_icon!(
    KEYBOARD,
    "keyboard",
    r#"<rect x="2.5" y="6" width="19" height="12" rx="2"/><path d="M6.5 10h.01M10 10h.01M14 10h.01M17.5 10h.01M8 14h8"/>"#
);
line_icon!(
    PANEL,
    "panel",
    r#"<rect x="2.5" y="4" width="19" height="14" rx="2"/><path d="M8 20h8M7 8.5h10"/>"#
);
line_icon!(
    LANGUAGES,
    "languages",
    r#"<path d="M4 6h8M8 4v2M10.5 6c-.6 3.4-2.8 6.2-6 7.5M6 9c1 2 2.7 3.6 5 4.5"/><path d="M12.5 20l3.5-8 3.5 8M13.7 17.5h4.6"/>"#
);
line_icon!(
    FILES,
    "files",
    r#"<path d="M14 3H8a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5M3 8v11a2 2 0 0 0 2 2h9"/>"#
);
line_icon!(
    CLIPBOARD,
    "clipboard",
    r#"<path d="M9 4.5H7a2 2 0 0 0-2 2V19a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V6.5a2 2 0 0 0-2-2h-2"/><rect x="9" y="3" width="6" height="3" rx="1"/>"#
);
line_icon!(
    FOLDER,
    "folder",
    r#"<path d="M3.5 7a2 2 0 0 1 2-2h4l2 2.5h7a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/>"#
);
line_icon!(
    LOGIN,
    "login",
    r#"<path d="M13 4h5a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-5M4 12h11m-4-4 4 4-4 4"/>"#
);
line_icon!(
    REFRESH,
    "refresh",
    r#"<path d="M20 11a8 8 0 0 0-14.3-4.9L4 8M4 4v4h4M4 13a8 8 0 0 0 14.3 4.9L20 16M20 20v-4h-4"/>"#
);
line_icon!(
    INFO,
    "info",
    r#"<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5M12 8h.01"/>"#
);
line_icon!(
    ACCESSIBILITY,
    "accessibility",
    r#"<circle cx="12" cy="4.5" r="1.5"/><path d="M5 8.5l7 1.5 7-1.5M12 10v4.5M12 14.5L9 20M12 14.5l3 5.5"/>"#
);
line_icon!(
    CALENDAR,
    "calendar",
    r#"<rect x="3.5" y="5" width="17" height="15" rx="2"/><path d="M3.5 10h17M8 3v4M16 3v4"/>"#
);

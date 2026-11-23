use dupin_core::Backdrop;
use dupin_core::config::{Colors, FontFamily, Theme};
use rune::{El, PopupItem, PopupMenu, TextStyle, Tx, col, el, icon, row, text};
use rune_kit::{Kit, SegmentedControl, State};

use super::Prefs;
use super::controls::{
    ROW_PX, anchor, card, lock_note, menu_value, setting, setting_with_lock, tilde, value_button,
};
use crate::icons;
use crate::shader::EXAMPLES;

const BACKDROP: &str = "backdrop";
const FLOW: &str = "flow";
const OFF: &str = "off";
const EXAMPLE: &str = "example:";
const FILE: &str = "file:";

pub(super) fn page(s: &Prefs, k: &Kit, dark: bool) -> Vec<El> {
    // The menu lists the shaders folder as it stands when it opens, so the trigger shows only
    // the current choice.
    let locked = s.locked("backdrop");
    let backdrop = menu_value(
        locked,
        k,
        BACKDROP,
        "Backdrop",
        &backdrop_title(&s.config.backdrop),
        pick_backdrop,
    );
    let dir = crate::shader::shaders_dir();
    let folder = tilde(&dir.display().to_string());
    let open = value_button(
        k,
        "open-shaders",
        "Open",
        &rune_theme_opencode::icons::CHEVRON_RIGHT,
    )
    .on_press(|_: &mut Prefs| open_shaders_folder());
    vec![
        theme_card(s, k, dark),
        card(
            k,
            1.0,
            vec![
                setting_with_lock(
                    k,
                    &icons::SPARKLES,
                    "Backdrop",
                    Some("The shader that animates behind the panel"),
                    backdrop,
                    s.lock_note(locked),
                ),
                setting(
                    k,
                    &icons::FOLDER,
                    "Shaders folder",
                    Some(&format!("Put .wgsl files in {folder} to pick them above")),
                    open,
                ),
            ],
        ),
    ]
}

/// The backdrop menu: the built-in shader, none, an example (copied beside config.toml to
/// be edited) or a file in the shaders folder. A file elsewhere that config.toml names shows
/// checked with them.
fn pick_backdrop(s: &mut Prefs, tx: &mut Tx) {
    let current = &s.config.backdrop;
    let id = |b: &Backdrop| match b {
        Backdrop::Flow => FLOW.to_owned(),
        Backdrop::Off => OFF.to_owned(),
        Backdrop::File(path) => match example_of(path) {
            Some(name) => format!("{EXAMPLE}{name}"),
            None => format!("{FILE}{path}"),
        },
    };
    let selected = id(current);
    let item = |b: Backdrop| {
        let id = id(&b);
        PopupItem::new(id.clone(), backdrop_title(&b)).checked(id == selected)
    };
    let mut files = crate::shader::user_shaders();
    if let Backdrop::File(path) = current
        && example_of(path).is_none()
        && !files.contains(path)
    {
        files.push(path.clone());
    }
    let mut menu = PopupMenu::new()
        .item(item(Backdrop::Flow))
        .item(item(Backdrop::Off))
        .separator()
        .items(
            EXAMPLES
                .iter()
                .map(|(name, _, _)| item(Backdrop::File(crate::shader::example_pick(name)))),
        );
    if !files.is_empty() {
        menu = menu
            .separator()
            .items(files.into_iter().map(|f| item(Backdrop::File(f))));
    }
    tx.popup_menu(
        menu.selected(selected),
        anchor(BACKDROP),
        |s: &mut Prefs, picked, _| {
            let Some(picked) = picked else { return };
            let next = if picked == FLOW {
                Backdrop::Flow
            } else if picked == OFF {
                Backdrop::Off
            } else if let Some(name) = picked.strip_prefix(EXAMPLE) {
                match crate::shader::install_example(name) {
                    Ok(next) => next,
                    Err(e) => return crate::log::warn(&format!("couldn't copy the {name} shader: {e}")),
                }
            } else if let Some(path) = picked.strip_prefix(FILE) {
                Backdrop::File(path.to_owned())
            } else {
                return;
            };
            s.set(|c| c.backdrop = next);
        },
    );
}

fn open_shaders_folder() {
    std::thread::spawn(|| {
        let dir = crate::shader::shaders_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            crate::log::warn(&format!("couldn't create {}: {e}", dir.display()));
            return;
        }
        _ = dupin_mac::workspace::open(&dir);
    });
}

/// The example a `backdrop` path is Settings' copy of.
fn example_of(path: &str) -> Option<&'static str> {
    EXAMPLES
        .iter()
        .map(|(name, _, _)| *name)
        .find(|name| crate::shader::example_pick(name) == path)
}

fn backdrop_title(backdrop: &Backdrop) -> String {
    match backdrop {
        Backdrop::Flow => "Flow".to_owned(),
        Backdrop::Off => "Off".to_owned(),
        Backdrop::File(path) => match example_of(path) {
            Some(name) => EXAMPLES
                .iter()
                .find(|(n, _, _)| *n == name)
                .map(|(_, t, _)| (*t).to_owned())
                .unwrap_or_default(),
            None => {
                let name = path.rsplit('/').next().unwrap_or(path);
                name.strip_suffix(".wgsl").unwrap_or(name).to_owned()
            }
        },
    }
}

type Palette = (&'static str, &'static str, &'static str);

const PRESETS: [(&str, Option<Palette>); 3] = [
    ("Default", None),
    ("Ocean", Some(("#E9F4FA", "#142734", "#176EA6"))),
    ("Rose", Some(("#FFF0F5", "#30202A", "#A52D61"))),
];

fn preset_colors(i: usize) -> (Colors, Colors) {
    let Some((light, dark, accent)) = PRESETS[i].1 else {
        return (Colors::default(), Colors::default());
    };
    let colors = |bg: &str| Colors {
        background: Some(bg.into()),
        accent: Some(accent.into()),
        ..Colors::default()
    };
    (colors(light), colors(dark))
}

fn color_preset(t: &Theme) -> Option<usize> {
    (0..PRESETS.len()).find(|&i| {
        let (light, dark) = preset_colors(i);
        t.light == light && t.dark == dark
    })
}

fn set_font(t: &mut Theme, size: u16) {
    t.font_size = size;
    if size == 18 {
        t.row_height = t.row_height.max(61);
    }
}

fn set_rows(t: &mut Theme, height: u16) {
    t.row_height = height;
    if height == 52 {
        t.font_size = t.font_size.min(15);
    }
}

fn segments(
    locked: bool,
    k: &Kit,
    key: &'static str,
    label: &'static str,
    labels: &[&str],
    selected: Option<usize>,
    apply: impl Fn(&mut Theme, usize) + Clone + 'static,
) -> El {
    let mut control = SegmentedControl::new(labels, selected).label(label).fit();
    if locked {
        for i in 0..labels.len() {
            control = control.segment_state(i, State::Disabled);
        }
    } else {
        control = control.on_pick(move |s: &mut Prefs, i, _| {
            let apply = apply.clone();
            s.set(move |c| apply(&mut c.theme, i));
        });
    }
    control.build(k).key(key)
}

fn theme_row(s: &Prefs, k: &Kit, name: &str, controls: El, locked: bool) -> El {
    row()
        .w_full()
        .h(40.0)
        .px(ROW_PX)
        .center_y()
        .gap(12.0)
        .child(text(name).font(k.ty.small).color(k.c.text_base).grow(1.0))
        .children(s.lock_note(locked).map(|note| lock_note(k, note)))
        .child(controls)
}

fn theme_card(s: &Prefs, k: &Kit, dark: bool) -> El {
    let locked = s.locked("theme");
    let t = &s.config.theme;
    let selected = color_preset(t);
    let mut swatches = row()
        .role("radiogroup")
        .label("Palette")
        .focus_group()
        .gap(12.0)
        .center_y();
    for (i, (name, _)) in PRESETS.iter().enumerate() {
        let (light, dark) = preset_colors(i);
        let sample = Theme {
            light,
            dark,
            ..Theme::default()
        };
        let palette = crate::view::ResolvedTheme::new(&sample, false);
        let mut swatch = el()
            .key(format!("color-{i}"))
            .label(*name)
            .role("radio")
            .focusable(!locked)
            .focus_selected(selected == Some(i))
            .checked(selected == Some(i))
            .position_in_set(i + 1, PRESETS.len())
            .size(26.0, 26.0)
            .radius(13.0)
            .padding(3.0)
            .border(
                2.0,
                if selected == Some(i) {
                    k.c.border_focus
                } else {
                    rune::Color::TRANSPARENT
                },
            )
            .child(
                el().size(20.0, 20.0)
                    .radius(10.0)
                    .fill(palette.card)
                    .center()
                    .child(el().size(9.0, 9.0).radius(4.5).fill(palette.accent)),
            );
        if locked {
            swatch = swatch.disabled(true).layer_opacity(0.5);
        } else {
            swatch = swatch
                .focus_ring(2.0, 2.0, k.c.border_focus)
                .on_press(move |s: &mut Prefs| {
                    s.set(|c| {
                        (c.theme.light, c.theme.dark) = preset_colors(i);
                    })
                });
        }
        swatches = swatches.child(swatch);
    }
    if selected.is_none() {
        swatches = swatches.child(
            text("Custom")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .px(8.0)
                .py(4.0)
                .radius(6.0)
                .border(1.0, k.c.border_muted),
        );
    }
    let toggle = SegmentedControl::new(
        &["Light", "Dark", "System"],
        Some(match s.preview_dark {
            Some(false) => 0,
            Some(true) => 1,
            None => 2,
        }),
    )
    .label("Preview appearance")
    .fit()
    .on_pick(|s: &mut Prefs, i, _| {
        s.preview_dark = match i {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    })
    .build(k)
    .key("preview-appearance");
    let well = col()
        .w_full()
        .h(156.0)
        .padding(10.0)
        .gap(6.0)
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .child(
            row()
                .w_full()
                .h(28.0)
                .center_y()
                .child(
                    text(format!(
                        "{} × {} · {} pt · {} pt corners",
                        t.panel_width, t.row_height, t.font_size, t.corner_radius
                    ))
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .grow(1.0),
                )
                .child(toggle),
        )
        .child(el().w_full().grow(1.0).center().child(crate::view::theme_preview(
            t,
            s.preview_dark.unwrap_or(dark),
            414.0,
        )));
    let mut reset = value_button(k, "reset-theme", "Reset", &icons::REFRESH);
    if locked || *t == Theme::default() {
        reset = reset.disabled(true).layer_opacity(0.5);
    } else {
        reset = reset.on_press(|s: &mut Prefs| s.set(|c| c.theme = Theme::default()));
    }
    let mut corners = row().gap(10.0).center_y();
    if t.corner_radius != s.active_radius {
        corners = corners.child(
            text("Relaunch to apply")
                .font(TextStyle::new(11.0, 440.0))
                .color(k.c.text_muted),
        );
    }
    corners = corners.child(segments(
        locked,
        k,
        "theme-corners",
        "Corners",
        &["Square", "14", "28"],
        [0, 14, 28].iter().position(|&v| v == t.corner_radius),
        |t, i| t.corner_radius = [0, 14, 28][i],
    ));
    card(
        k,
        1.0,
        vec![
            row()
                .w_full()
                .h(40.0)
                .px(ROW_PX)
                .gap(8.0)
                .center_y()
                .child(icon(&icons::SPARKLES, 16.0).color(k.c.text_muted))
                .child(text("Theme").font(k.ty.small).color(k.c.text_base).grow(1.0)),
            el().w_full().px(ROW_PX).mb(4.0).child(well),
            theme_row(s, k, "Palette", swatches, locked),
            theme_row(
                s,
                k,
                "Font",
                row()
                    .gap(8.0)
                    .child(segments(
                        locked,
                        k,
                        "theme-font",
                        "Font",
                        &["System", "Mono"],
                        Some(usize::from(t.font_family == FontFamily::Monospace)),
                        |t, i| t.font_family = [FontFamily::System, FontFamily::Monospace][i],
                    ))
                    .child(segments(
                        locked,
                        k,
                        "theme-size",
                        "Font size",
                        &["15", "18"],
                        [15, 18].iter().position(|&v| v == t.font_size),
                        |t, i| set_font(t, [15, 18][i]),
                    )),
                locked,
            ),
            theme_row(
                s,
                k,
                "Width",
                segments(
                    locked,
                    k,
                    "theme-width",
                    "Width",
                    &["Narrow", "Regular", "Wide"],
                    [400, 680, 900].iter().position(|&v| v == t.panel_width),
                    |t, i| t.panel_width = [400, 680, 900][i],
                ),
                locked,
            ),
            theme_row(
                s,
                k,
                "Rows",
                segments(
                    locked,
                    k,
                    "theme-rows",
                    "Rows",
                    &["Compact", "Comfy", "Spacious"],
                    [52, 72, 100].iter().position(|&v| v == t.row_height),
                    |t, i| set_rows(t, [52, 72, 100][i]),
                ),
                locked,
            ),
            theme_row(s, k, "Corners", corners, locked),
        ],
    )
    .child(reset.pin(None, Some(6.0), Some(ROW_PX), None))
}

#[cfg(test)]
mod theme_control_tests {
    use super::*;

    #[test]
    fn presets_match_only_complete_color_overrides() {
        let mut theme = Theme::default();
        assert_eq!(color_preset(&theme), Some(0));
        (theme.light, theme.dark) = preset_colors(1);
        theme.panel_width = 1200;
        assert_eq!(color_preset(&theme), Some(1));
        theme.dark.text = Some("#FFFFFF".into());
        assert_eq!(color_preset(&theme), None);
        theme = Theme::default();
        theme.light.background = Some("#123456".into());
        assert_eq!(color_preset(&theme), None);
    }

    #[test]
    fn compact_rows_and_large_fonts_remain_compatible() {
        let mut theme = Theme::default();
        set_font(&mut theme, 18);
        assert_eq!(theme.row_height, 61);
        set_rows(&mut theme, 52);
        assert_eq!(theme.font_size, 15);
        theme.row_height = 100;
        set_font(&mut theme, 18);
        assert_eq!(theme.row_height, 100);
    }
}

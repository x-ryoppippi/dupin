use rune::tok;
use rune::{Color, El, TextStyle, col, el, row, text};

use crate::ui::ICON_SIZE;

use super::header::HEADER_H;
use super::results::{LIST_PAD, RowLabels, text_row};

pub struct ResolvedTheme {
    palette: tok::Theme,
    pub accent: Color,
    pub selected_text: Color,
    pub destructive: Color,
    pub warning: Color,
    scale: f32,
    family: tok::Family,
}
impl std::ops::Deref for ResolvedTheme {
    type Target = tok::Theme;
    fn deref(&self) -> &Self::Target {
        &self.palette
    }
}
impl ResolvedTheme {
    pub fn new(theme: &dupin_core::config::Theme, dark: bool) -> Self {
        let c = if dark { &theme.dark } else { &theme.light };
        let mut palette = tok::theme(dark);
        fn color(value: &Option<String>, fallback: Color) -> Color {
            value
                .as_ref()
                .and_then(|v| rune::color::hex(v))
                .unwrap_or(fallback)
        }
        palette.card = color(&c.background, palette.card);
        palette.text = color(&c.text, palette.text);
        palette.text_secondary = color(&c.secondary, palette.text_secondary);
        palette.hover = color(&c.hover, palette.hover);
        palette.separator = color(&c.separator, palette.separator);
        Self {
            palette,
            accent: color(&c.accent, tok::accent(0, dark)),
            selected_text: color(&c.selected_text, Color::WHITE),
            destructive: rune::color::hex(if dark { "#FF453A" } else { "#FF3B30" }).unwrap(),
            warning: rune::color::hex(if dark { "#FF9F0A" } else { "#C93400" }).unwrap(),
            scale: theme.font_size as f32 / 15.0,
            family: match &theme.font_family {
                dupin_core::config::FontFamily::System => tok::Family::Sans,
                dupin_core::config::FontFamily::Monospace => tok::Family::SystemMono,
                dupin_core::config::FontFamily::Named(name) => tok::Family::Named(intern(name)),
            },
        }
    }
    pub fn font(&self, mut style: TextStyle) -> TextStyle {
        style.size *= self.scale;
        style.line_height *= self.scale;
        style.family = self.family;
        style
    }
}

/// `tok::Family::Named` holds a `&'static str`: leak each distinct configured name once.
fn intern(name: &str) -> &'static str {
    static NAMES: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(n) = names.iter().find(|n| **n == name) {
        return n;
    }
    let n: &'static str = Box::leak(name.into());
    names.push(n);
    n
}

fn preview_geometry(theme: &dupin_core::config::Theme, avail_w: f32) -> (f32, f32, f32, usize) {
    let n = if theme.row_height <= 72 { 3 } else { 2 };
    let h = HEADER_H + 1.0 + 2.0 * LIST_PAD + n as f32 * theme.row_height as f32;
    let w = theme.panel_width as f32;
    // Reserve room for the view toggle and exact-value caption in the well.
    let scale = (avail_w / w).min(104.0 / h).min(0.5);
    (w, h, scale, n)
}

/// How far the miniature's glass blurs the desk behind it, in the panel's own points.
const GLASS_BLUR: f32 = 24.0;
/// How much of the card colour covers the miniature's glass: the system glass's own frost
/// plus the panel's `FROST` over it.
const GLASS_TINT: f32 = 0.6;

/// The miniature panel on a small desk of colour, so its glass has something to show through it.
/// The desk spans the width it is given; the caller sizes its height, rounds and clips it.
pub fn theme_preview(theme: &dupin_core::config::Theme, dark: bool, avail_w: f32) -> El {
    let t = ResolvedTheme::new(theme, dark);
    let (w, h, scale, n) = preview_geometry(theme, avail_w);
    let radius = theme.corner_radius as f32;
    let ink = if dark { Color::WHITE } else { Color::BLACK };
    let panel = col()
        .w(w)
        .h(h)
        .radius(radius)
        .continuous_corners()
        .clip_rounded(radius)
        .backdrop_blur(GLASS_BLUR)
        .fill(t.card.alpha(GLASS_TINT))
        .border_gradient(
            1.0,
            Color::WHITE.alpha(if dark { 0.22 } else { 0.7 }),
            ink.alpha(0.08),
        )
        .shadow(10.0, 32.0, Color::BLACK.alpha(if dark { 0.35 } else { 0.16 }))
        .child(
            row()
                .w_full()
                .h(HEADER_H)
                .px(18.0)
                .gap(12.0)
                .center_y()
                .child(
                    text("⌕")
                        .font(t.font(TextStyle::new(24.0, 400.0).lh(30.0)))
                        .color(t.text_secondary),
                )
                .child(
                    text("gh dupin")
                        .font(t.font(TextStyle::new(22.0, 400.0).lh(30.0)))
                        .color(t.text),
                )
                .child(el().size(2.0, 26.0).fill(t.accent)),
        )
        .child(divider(&t))
        .child(col().w_full().py(LIST_PAD).children((0..n).map(|i| {
            let on = i == 0;
            text_row(
                &t,
                t.accent,
                theme.row_height as f32,
                on,
                Some(
                    el().size(ICON_SIZE, ICON_SIZE)
                        .radius(4.0)
                        .fill(t.text_secondary.alpha(0.25)),
                ),
                RowLabels {
                    title: text(["dupin", "dupin-core", "dupin-mac"][i])
                        .font(t.font(TextStyle::new(15.0, 500.0).lh(20.0)))
                        .color(if on { t.selected_text } else { t.text })
                        .truncate(),
                    subtitle: "Repository · → Actions",
                    accessories: &[],
                },
            )
        })));
    let miniature = el()
        .size((w * scale).round(), (h * scale).round())
        .child(panel.scale(scale).pin(
            Some((w * scale - w) / 2.0),
            Some((h * scale - h) / 2.0),
            None,
            None,
        ));
    desk(&t, dark).a11y_hidden().child(miniature)
}

/// A soft wallpaper of the theme's accent and two fixed hues, drawn once: Settings has no
/// backdrop compositor, and a still desk keeps the window idle while it is open.
fn desk(t: &ResolvedTheme, dark: bool) -> El {
    let (l, top, bottom) = if dark {
        (
            0.55,
            rune::oklch(0.27, 0.04, 265.0),
            rune::oklch(0.22, 0.05, 320.0),
        )
    } else {
        (0.8, rune::oklch(0.94, 0.03, 250.0), rune::oklch(0.9, 0.04, 330.0))
    };
    let blob = |center: [f32; 2], radius: [f32; 2], color: Color| {
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radial_gradient(center, radius, [(0.0, color), (1.0, color.alpha(0.0))])
    };
    el().w_full()
        .center()
        .gradient(top, bottom)
        .child(blob([0.22, 0.8], [190.0, 110.0], t.accent.alpha(0.85)))
        .child(blob([0.8, 0.15], [170.0, 100.0], rune::oklch(l, 0.14, 45.0)))
        .child(blob(
            [0.62, 1.0],
            [150.0, 80.0],
            rune::oklch(l - 0.05, 0.16, 300.0),
        ))
}

pub(super) fn divider(t: &ResolvedTheme) -> El {
    el().h(1.0).w_full().fill(t.separator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miniature_fits_both_axes_for_custom_geometry() {
        for width in [400, 680, 900, 1200] {
            for height in [52, 72, 100] {
                let theme = dupin_core::config::Theme {
                    panel_width: width,
                    row_height: height,
                    ..Default::default()
                };
                let (w, h, scale, n) = preview_geometry(&theme, 414.0);
                assert!(w * scale <= 414.01);
                assert!(h * scale <= 104.01);
                assert!(scale <= 0.5);
                assert_eq!(n, if height <= 72 { 3 } else { 2 });
            }
        }
    }

    #[test]
    fn defaults_and_mode_overrides_resolve_independently() {
        let mut theme = dupin_core::config::Theme::default();
        assert_eq!(ResolvedTheme::new(&theme, false).text, tok::theme(false).text);
        assert_eq!(ResolvedTheme::new(&theme, true).card, tok::theme(true).card);
        theme.light.text = Some("#123456".into());
        assert_ne!(ResolvedTheme::new(&theme, false).text, tok::theme(false).text);
        assert_eq!(ResolvedTheme::new(&theme, true).text, tok::theme(true).text);
        theme.font_size = 18;
        theme.font_family = dupin_core::config::FontFamily::Monospace;
        let font = ResolvedTheme::new(&theme, false).font(TextStyle::new(15.0, 400.0).lh(20.0));
        assert_eq!(font.size, 18.0);
        assert_eq!(font.line_height, 24.0);
        assert_eq!(font.family, tok::Family::SystemMono);
        theme.font_family = dupin_core::config::FontFamily::Named("Menlo".into());
        let font = ResolvedTheme::new(&theme, false).font(TextStyle::new(15.0, 400.0).lh(20.0));
        assert_eq!(font.family, tok::Family::Named("Menlo"));
    }
}

use dupin_core::Item;
use dupin_core::detail::{Block, Detail, Inline};
use rune::tok::{Family, type_};
use rune::{Color, El, Link, ScrollSpec, Span, TextStyle, col, el, rich, row, text};

use crate::ui::{Preview, Ui};

use super::results::{card, card_slot, row_key};
use super::theme::ResolvedTheme;

const BODY: TextStyle = TextStyle::new(15.0, 400.0).lh(23.0);
const METADATA_LABEL_W: f32 = 120.0;

pub(super) fn detail_card(
    s: &Ui,
    item: &Item,
    detail: &Detail,
    index: usize,
    on: bool,
    t: &ResolvedTheme,
    accent: Color,
) -> El {
    let key = row_key(&s.session.layout_key(index));
    let mut content = col().key(format!("{key}-detail")).w_full().gap(14.0);
    if let Some(image) = &detail.image {
        let slot = || {
            el().key(format!("{key}-image"))
                .w_full()
                .h(crate::ui::PREVIEW_HEIGHT)
                .radius(6.0)
        };
        match s.thumbnails.preview(image) {
            Preview::Ready(preview) => {
                content = content.child(slot().pixels_fit(preview.pixels(), rune::SurfaceFit::Fit));
            }
            Preview::Loading => content = content.child(slot()),
            Preview::Failed => {}
        }
    }
    if !detail.metadata.is_empty() {
        content = content.child(
            col()
                .key(format!("{key}-metadata"))
                .w_full()
                .gap(6.0)
                .selection_group(true)
                .children(detail.metadata.iter().enumerate().map(|(i, entry)| {
                    row()
                        .w_full()
                        .gap(12.0)
                        .child(
                            text(&entry.label)
                                .w(METADATA_LABEL_W)
                                .font(t.font(type_::LABEL.lh(20.0)))
                                .color(t.text_secondary)
                                .truncate(),
                        )
                        .child(
                            text(&entry.value)
                                .key(format!("{key}-metadata-{i}"))
                                .grow(1.0)
                                .min_w(0.0)
                                .font(t.font(TextStyle::new(14.0, 400.0).lh(20.0)))
                                .color(t.text)
                                .wrap()
                                .selectable(true),
                        )
                })),
        );
        if !detail.body.blocks.is_empty() {
            content = content.child(el().h(1.0).w_full().fill(t.separator));
        }
    }
    let renderer = Renderer {
        t,
        accent,
        result: s.session.result_key(index),
    };
    let mut body = col()
        .key(format!("{key}-selection"))
        .w_full()
        .gap(12.0)
        .selection_group(true)
        .children(renderer.blocks(&format!("{key}-md"), &detail.body.blocks));
    if detail.body.truncated {
        body = body.child(text("…").font(t.font(BODY)).color(t.text_secondary));
    }
    content = content.child(body);
    card_slot(
        card(s, item, index, on, Some(&detail.title), t, accent).child(
            col()
                .key(format!("{key}-body"))
                .w_full()
                .shrink()
                .min_h(20.0)
                .max_h(340.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(content),
        ),
    )
}

struct Renderer<'a> {
    t: &'a ResolvedTheme,
    accent: Color,
    /// The result key the detail belongs to, so a pressed link opens only while it is shown.
    result: String,
}

impl Renderer<'_> {
    fn blocks(&self, key: &str, blocks: &[Block]) -> Vec<El> {
        blocks
            .iter()
            .enumerate()
            .map(|(i, block)| self.block(&format!("{key}-{i}"), block))
            .collect()
    }

    fn block(&self, key: &str, block: &Block) -> El {
        let t = self.t;
        match block {
            Block::Heading { level, text } => {
                let style = match level {
                    1 => TextStyle::new(20.0, 600.0).lh(26.0),
                    2 => TextStyle::new(17.0, 600.0).lh(23.0),
                    _ => TextStyle::new(15.0, 600.0).lh(21.0),
                };
                self.inline(key, text, style).role("heading").mt(4.0)
            }
            Block::Paragraph(text) => self.inline(key, text, BODY),
            Block::List { start, items } => {
                col()
                    .key(key.to_owned())
                    .w_full()
                    .gap(6.0)
                    .children(items.iter().enumerate().map(|(i, item)| {
                        let marker = match start {
                            Some(start) => format!("{}.", start.saturating_add(i as u64)),
                            None => "•".into(),
                        };
                        row()
                            .w_full()
                            .gap(8.0)
                            .child(
                                text(marker)
                                    .w(22.0)
                                    .font(t.font(BODY))
                                    .color(t.text_secondary)
                                    .align(rune::Align::End),
                            )
                            .child(
                                col()
                                    .grow(1.0)
                                    .min_w(0.0)
                                    .gap(6.0)
                                    .children(self.blocks(&format!("{key}-{i}"), item)),
                            )
                    }))
            }
            Block::Code(code) => col()
                .key(key.to_owned())
                .w_full()
                .px(12.0)
                .py(10.0)
                .radius(6.0)
                .fill(code_fill(t))
                .child(
                    text(code)
                        .key(format!("{key}-code"))
                        .w_full()
                        .font(mono(t.font(TextStyle::new(13.0, 400.0).lh(19.0))))
                        .color(t.text)
                        .wrap()
                        .selectable(true),
                ),
        }
    }

    fn inline(&self, key: &str, inline: &Inline, style: TextStyle) -> El {
        let t = self.t;
        let style = t.font(style);
        let spans = inline
            .runs
            .iter()
            .map(|run| {
                let mut span = Span::new(run.range.start, run.range.end);
                if run.style.strong {
                    span = span.bold();
                }
                if run.style.emphasis {
                    span = span.italic();
                }
                if run.style.code {
                    span = span.mono().size(style.size * 0.88).bg(code_fill(t), 3.0);
                }
                if run.style.link.is_some() {
                    span = span.color(self.accent).underline(self.accent.alpha(0.45));
                }
                span
            })
            .collect();
        let mut el = rich(inline.text.clone(), spans)
            .key(key.to_owned())
            .w_full()
            .font(style)
            .color(t.text)
            .selectable(true);
        let links = inline.links();
        if !links.is_empty() {
            for (range, url) in links {
                el = el.link(Link::new(range, url).help(url));
            }
            let result = self.result.clone();
            el = el.on_link(move |s: &mut Ui, url: &str, tx| s.open_link(&result, url, tx));
        }
        el
    }
}

fn code_fill(t: &ResolvedTheme) -> Color {
    t.text.alpha(0.07)
}

fn mono(mut style: TextStyle) -> TextStyle {
    style.family = Family::SystemMono;
    style
}

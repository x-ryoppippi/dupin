//! A result shown as a page of its own: a title, labeled metadata and a Markdown body that the
//! host parses once into a bounded document and renders itself.

use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

/// Markdown past this many bytes is dropped before parsing.
const MAX_MARKDOWN_BYTES: usize = 32 * 1024;
/// Blocks past this many are dropped, so a document stays cheap to lay out.
const MAX_BLOCKS: usize = 512;
/// Lists nested deeper than this are flattened into their deepest allowed item.
const MAX_LIST_DEPTH: usize = 3;
const MAX_METADATA: usize = 12;
const MAX_LINK_BYTES: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detail {
    pub title: String,
    pub metadata: Vec<Metadata>,
    pub body: Document,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub label: String,
    pub value: String,
}

impl Detail {
    pub fn new(title: impl Into<String>, markdown: &str) -> Self {
        Self {
            title: title.into(),
            metadata: Vec::new(),
            body: Document::parse(markdown),
        }
    }

    /// Add a labeled value above the body; values past [`MAX_METADATA`] are dropped.
    pub fn with_metadata(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        let value = value.into();
        if self.metadata.len() < MAX_METADATA && !value.trim().is_empty() {
            self.metadata.push(Metadata {
                label: label.into(),
                value,
            });
        }
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
    /// The source passed a size or block limit and its end is missing.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// Levels 1 to 6, as in Markdown.
    Heading {
        level: u8,
        text: Inline,
    },
    Paragraph(Inline),
    /// `start` is the first number of an ordered list.
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Code(String),
}

/// Text with styled byte ranges. Ranges are sorted and never overlap.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub range: Range<usize>,
    pub style: Style,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub strong: bool,
    pub emphasis: bool,
    pub code: bool,
    /// An http or https URL that the host opens when the range is pressed.
    pub link: Option<String>,
}

impl Inline {
    fn push(&mut self, text: &str, style: &Style) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        if *style == Style::default() {
            return;
        }
        match self.runs.last_mut() {
            Some(last) if last.range.end == start && last.style == *style => last.range.end = self.text.len(),
            _ => self.runs.push(Run {
                range: start..self.text.len(),
                style: style.clone(),
            }),
        }
    }

    /// Each link's whole range, even where its styling changes partway through.
    pub fn links(&self) -> Vec<(Range<usize>, &str)> {
        let mut links: Vec<(Range<usize>, &str)> = Vec::new();
        for run in &self.runs {
            let Some(url) = run.style.link.as_deref() else {
                continue;
            };
            match links.last_mut() {
                Some((range, last)) if range.end == run.range.start && *last == url => {
                    range.end = run.range.end
                }
                _ => links.push((run.range.clone(), url)),
            }
        }
        links
    }
}

impl Document {
    fn parse(markdown: &str) -> Self {
        let mut end = markdown.len().min(MAX_MARKDOWN_BYTES);
        while !markdown.is_char_boundary(end) {
            end -= 1;
        }
        let mut builder = Builder {
            truncated: end < markdown.len(),
            ..Builder::default()
        };
        for event in Parser::new(&markdown[..end]) {
            if builder.blocks >= MAX_BLOCKS {
                builder.truncated = true;
                break;
            }
            builder.event(event);
        }
        builder.finish()
    }

    /// Whether `url` is the target of one of the document's links.
    pub(crate) fn links_to(&self, url: &str) -> bool {
        fn search(blocks: &[Block], url: &str) -> bool {
            blocks.iter().any(|block| match block {
                Block::Heading { text, .. } | Block::Paragraph(text) => {
                    text.runs.iter().any(|run| run.style.link.as_deref() == Some(url))
                }
                Block::List { items, .. } => items.iter().any(|item| search(item, url)),
                Block::Code(_) => false,
            })
        }
        search(&self.blocks, url)
    }
}

#[derive(Default)]
struct Builder {
    root: Vec<Block>,
    lists: Vec<(Option<u64>, Vec<Vec<Block>>)>,
    /// Lists opened past [`MAX_LIST_DEPTH`], whose items join the deepest open item.
    overflow: usize,
    inline: Option<Inline>,
    heading: Option<u8>,
    code: Option<String>,
    strong: usize,
    emphasis: usize,
    links: Vec<Option<String>>,
    blocks: usize,
    truncated: bool,
}

impl Builder {
    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => match &mut self.code {
                Some(code) => code.push_str(&text),
                None => self.text(&text, false),
            },
            Event::Code(text) => self.text(&text, true),
            Event::Html(text) | Event::InlineHtml(text) => self.text(&text, false),
            Event::SoftBreak => self.text(" ", false),
            Event::HardBreak => self.text("\n", false),
            Event::Rule => self.flush(),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph | Tag::HtmlBlock => {
                self.flush();
                self.inline = Some(Inline::default());
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading = Some(level as u8);
                self.inline = Some(Inline::default());
            }
            Tag::CodeBlock(_) => {
                self.flush();
                self.code = Some(String::new());
            }
            Tag::List(start) => {
                self.flush();
                if self.overflow > 0 || self.lists.len() >= MAX_LIST_DEPTH {
                    self.overflow += 1;
                } else {
                    self.lists.push((start, Vec::new()));
                }
            }
            Tag::Item => {
                self.flush();
                if self.overflow == 0
                    && let Some((_, items)) = self.lists.last_mut()
                {
                    items.push(Vec::new());
                    self.blocks += 1;
                }
            }
            Tag::Emphasis => self.emphasis += 1,
            Tag::Strong => self.strong += 1,
            Tag::Link { dest_url, .. } => self.links.push(web_link(&dest_url).map(str::to_owned)),
            Tag::BlockQuote(_) => self.flush(),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::HtmlBlock | TagEnd::Heading(_) | TagEnd::Item => self.flush(),
            TagEnd::CodeBlock => {
                if let Some(code) = self.code.take() {
                    self.push(Block::Code(code.trim_end_matches('\n').to_owned()));
                }
            }
            TagEnd::List(_) => {
                self.flush();
                if self.overflow > 0 {
                    self.overflow -= 1;
                } else if let Some((start, items)) = self.lists.pop() {
                    self.push(Block::List { start, items });
                }
            }
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Link => {
                self.links.pop();
            }
            TagEnd::BlockQuote(_) => self.flush(),
            _ => {}
        }
    }

    fn text(&mut self, text: &str, code: bool) {
        let style = Style {
            strong: self.strong > 0,
            emphasis: self.emphasis > 0,
            code,
            link: self.links.iter().rev().find_map(Clone::clone),
        };
        self.inline.get_or_insert_default().push(text, &style);
    }

    fn flush(&mut self) {
        let heading = self.heading.take();
        let Some(mut inline) = self.inline.take() else {
            return;
        };
        let trimmed = inline.text.trim_end().len();
        if trimmed == 0 {
            return;
        }
        inline.text.truncate(trimmed);
        inline.runs.retain_mut(|run| {
            run.range.end = run.range.end.min(trimmed);
            run.range.start < run.range.end
        });
        self.push(match heading {
            Some(level) => Block::Heading { level, text: inline },
            None => Block::Paragraph(inline),
        });
    }

    fn push(&mut self, block: Block) {
        self.blocks += 1;
        match self.lists.last_mut() {
            Some((_, items)) => {
                if items.is_empty() {
                    items.push(Vec::new());
                }
                items.last_mut().unwrap().push(block);
            }
            None => self.root.push(block),
        }
    }

    fn finish(mut self) -> Document {
        self.flush();
        if let Some(code) = self.code.take() {
            self.push(Block::Code(code.trim_end_matches('\n').to_owned()));
        }
        while let Some((start, items)) = self.lists.pop() {
            self.push(Block::List { start, items });
        }
        Document {
            blocks: self.root,
            truncated: self.truncated,
        }
    }
}

/// `url` if it is an http or https URL with a host, the only links a detail makes pressable.
pub(crate) fn web_link(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    ((scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http"))
        && !host.is_empty()
        && url.len() <= MAX_LINK_BYTES
        && !url.chars().any(|c| c.is_whitespace() || c.is_control()))
    .then_some(url)
}

/// `text` with every ASCII punctuation character escaped, so Markdown shows it as written.
pub fn escape_markdown(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph(block: &Block) -> &Inline {
        match block {
            Block::Paragraph(inline) => inline,
            other => panic!("expected a paragraph, got {other:?}"),
        }
    }

    #[test]
    fn headings_paragraphs_emphasis_and_code_keep_their_styles() {
        let doc =
            Document::parse("# Title\n\nSome **bold** and *slanted* `code`.\n\n```rust\nfn main() {}\n```\n");
        assert!(!doc.truncated);
        assert_eq!(doc.blocks.len(), 3);
        assert!(matches!(&doc.blocks[0], Block::Heading { level: 1, text } if text.text == "Title"));
        let text = paragraph(&doc.blocks[1]);
        assert_eq!(text.text, "Some bold and slanted code.");
        let styled: Vec<_> = text
            .runs
            .iter()
            .map(|run| {
                (
                    &text.text[run.range.clone()],
                    run.style.strong,
                    run.style.emphasis,
                    run.style.code,
                )
            })
            .collect();
        assert_eq!(
            styled,
            [
                ("bold", true, false, false),
                ("slanted", false, true, false),
                ("code", false, false, true),
            ]
        );
        assert_eq!(doc.blocks[2], Block::Code("fn main() {}".into()));
    }

    #[test]
    fn lists_hold_their_items_and_ordered_start() {
        let doc = Document::parse("3. first\n4. second\n   - nested\n");
        let [Block::List { start, items }] = doc.blocks.as_slice() else {
            panic!("expected one list: {:?}", doc.blocks);
        };
        assert_eq!(*start, Some(3));
        assert_eq!(items.len(), 2);
        assert_eq!(paragraph(&items[0][0]).text, "first");
        assert_eq!(paragraph(&items[1][0]).text, "second");
        assert!(matches!(&items[1][1], Block::List { start: None, items } if items.len() == 1));
    }

    #[test]
    fn lists_deeper_than_the_limit_flatten_into_the_deepest_item() {
        let doc = Document::parse("- a\n  - b\n    - c\n      - d\n        - e\n");
        let mut depth = 0;
        let mut blocks = &doc.blocks;
        while let Some(Block::List { items, .. }) = blocks.iter().find(|b| matches!(b, Block::List { .. })) {
            depth += 1;
            blocks = &items[0];
        }
        assert_eq!(depth, MAX_LIST_DEPTH);
        let texts: Vec<_> = blocks
            .iter()
            .map(|block| paragraph(block).text.as_str())
            .collect();
        assert_eq!(texts, ["c", "d", "e"]);
    }

    #[test]
    fn only_web_links_are_pressable() {
        let doc = Document::parse(
            "[site](https://example.com/a?b#c) [mail](mailto:a@example.com) [file](file:///etc/hosts) \
             [js](javascript:alert(1)) <http://example.org> [**bold** link](https://example.com/b)",
        );
        let text = paragraph(&doc.blocks[0]);
        let links: Vec<_> = text
            .links()
            .into_iter()
            .map(|(range, url)| (&text.text[range], url))
            .collect();
        assert_eq!(
            links,
            [
                ("site", "https://example.com/a?b#c"),
                ("http://example.org", "http://example.org"),
                ("bold link", "https://example.com/b"),
            ]
        );
        assert!(text.text.contains("mail") && text.text.contains("js"));
        assert!(doc.links_to("http://example.org"));
        assert!(!doc.links_to("mailto:a@example.com"));
        assert_eq!(web_link("HTTPS://example.com"), Some("HTTPS://example.com"));
        for url in [
            "https://",
            "https:///path",
            "https://a b",
            "ftp://example.com",
            "example.com",
        ] {
            assert_eq!(web_link(url), None, "{url}");
        }
    }

    #[test]
    fn raw_html_and_images_are_shown_as_inert_text() {
        let doc = Document::parse("<script>alert(1)</script>\n\n![alt text](https://example.com/i.png)");
        assert_eq!(paragraph(&doc.blocks[0]).text, "<script>alert(1)</script>");
        let image = paragraph(&doc.blocks[1]);
        assert_eq!(image.text, "alt text");
        assert!(image.runs.is_empty());
    }

    #[test]
    fn input_and_block_counts_are_bounded() {
        let long = "é".repeat(MAX_MARKDOWN_BYTES);
        let doc = Document::parse(&long);
        assert!(doc.truncated);
        assert_eq!(paragraph(&doc.blocks[0]).text.len(), MAX_MARKDOWN_BYTES);

        let many = "p\n\n".repeat(MAX_BLOCKS * 2);
        let doc = Document::parse(&many);
        assert!(doc.truncated);
        assert_eq!(doc.blocks.len(), MAX_BLOCKS);

        let quotes = format!("{}deep", ">".repeat(10_000));
        assert_eq!(paragraph(&Document::parse(&quotes).blocks[0]).text, "deep");
    }

    #[test]
    fn escaped_text_reads_back_unchanged() {
        let text = "# not a heading * [x](y) `z` 1. <b> \\ _u_";
        let doc = Document::parse(&escape_markdown(text));
        let inline = paragraph(&doc.blocks[0]);
        assert_eq!(inline.text, text);
        assert!(inline.runs.is_empty());
    }

    #[test]
    fn metadata_skips_blank_values_and_stays_bounded() {
        let mut detail = Detail::new("t", "").with_metadata("Empty", " ");
        for index in 0..MAX_METADATA + 4 {
            detail = detail.with_metadata("Label", index.to_string());
        }
        assert_eq!(detail.metadata.len(), MAX_METADATA);
        assert_eq!(detail.metadata[0].value, "0");
    }
}

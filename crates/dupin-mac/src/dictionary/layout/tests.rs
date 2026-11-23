use dupin_core::detail::{Block, Inline, Metadata};

use super::*;

/// Entries in the layouts of the dictionaries macOS ships, with original wording.
const ENGLISH: &str = "run | rʌn | verb (runs, running; past ran; past participle run) 1 [no object] move \
    on foot faster than walking: she ran to the station | he ran the last few yards. • (of a machine) \
    operate: the engine runs on diesel. • [with object] cause to operate: run the dishwasher twice a week. \
    2 [with object] be in charge of: he runs a small bakery | (-run as adjective) : a family-run hotel. \
    noun 1 an act of running: a morning run. 2 a journey on a regular route: the school run. PHRASES on \
    the run 1 trying to avoid being caught. 2 while running: he caught it on the run. [late 18th \
    century]ORIGIN Old English rinnan.";

const JAPANESE_ENGLISH: &str = "ねこ 【猫】 a cat ; ｟愛称｠ a puss | pʊs |, a pussy (cat) (!主に小児語) . \
    ▸ 三毛猫 a tortoiseshell cat.▸ 黒猫 a black cat.▸ 猫を飼っている have [keep] a cat.";

const ENGLISH_JAPANESE: &str = "hap･py | hǽpi | 形容詞-pier; -piest 1 〈人が〉うれしい, 楽しい (→ glad 2) \
    ▸ I'm happy to see you. 会えてうれしい.2 〖通例名詞の前で〗幸せな ▸ a happy ending 幸せな結末.";

const JAPANESE: &str = "はし・る 2【走る】（動ラ五）① 足をはやく動かして進む。「廊下を―・る」② 乗り物が進む。\
    〈枕草子•12〉③ 液体が動く。㋐ はやく流れる。㋑ 飛びはねる。";

const THESAURUS: &str = "happy adjective 1 she looked happy and relaxed. cheerful, merry, joyful, glad; \
    informal chirpy. ANTONYMS sad. 2 we are happy to help. willing, glad, ready. ANTONYMS unwilling. \
    WORD LINKS hedonic: relating to pleasure";

/// The body as indented lines: headings with `#`, list items with their markers, and italic
/// ranges between underscores.
fn outline(blocks: &[Block]) -> Vec<String> {
    fn styled(inline: &Inline) -> String {
        let mut text = String::new();
        let mut at = 0;
        for run in &inline.runs {
            text.push_str(&inline.text[at..run.range.start]);
            let part = &inline.text[run.range.clone()];
            match (run.style.emphasis, run.style.strong) {
                (true, _) => text.push_str(&format!("_{part}_")),
                (false, true) => text.push_str(&format!("**{part}**")),
                _ => text.push_str(part),
            }
            at = run.range.end;
        }
        text.push_str(&inline.text[at..]);
        text
    }
    fn walk(blocks: &[Block], indent: usize, lines: &mut Vec<String>) {
        for block in blocks {
            match block {
                Block::Heading { level, text } => {
                    lines.push(format!("{}{}", "#".repeat(usize::from(*level)), styled(text)))
                }
                Block::Paragraph(text) => lines.push(format!("{:indent$}{}", "", styled(text))),
                Block::List { start, items } => {
                    for (i, item) in items.iter().enumerate() {
                        let marker = match start {
                            Some(start) => format!("{}.", start + i as u64),
                            None => "-".into(),
                        };
                        let mut inner = Vec::new();
                        walk(item, indent + 2, &mut inner);
                        let first = inner.first().map_or("", |line| line.trim_start()).to_owned();
                        lines.push(format!("{:indent$}{marker} {first}", ""));
                        lines.extend(inner.into_iter().skip(1));
                    }
                }
                Block::Code(code) => lines.push(code.clone()),
            }
        }
    }
    let mut lines = Vec::new();
    walk(blocks, 0, &mut lines);
    lines
}

/// Every word of `definition` appears in the detail, apart from what became structure: sense
/// numbers and markers, the bars around the pronunciation, and the headword the title shows.
fn assert_keeps_every_word(term: &str, definition: &str, headword: &str) {
    let detail = detail(term, definition);
    let mut shown = detail
        .metadata
        .iter()
        .map(|entry| entry.value.clone())
        .collect::<Vec<_>>();
    fn texts(blocks: &[Block], into: &mut Vec<String>) {
        for block in blocks {
            match block {
                Block::Heading { text, .. } | Block::Paragraph(text) => into.push(text.text.clone()),
                Block::List { items, .. } => items.iter().for_each(|item| texts(item, into)),
                Block::Code(code) => into.push(code.clone()),
            }
        }
    }
    texts(&detail.body.blocks, &mut shown);
    let shown = shown.join(" ").to_lowercase();
    let markers = |c: char| matches!(c, '•' | '▸' | '|' | '\u{32D0}'..='\u{32FE}') || CIRCLED.contains(&c);
    let definition = PARTS.iter().fold(definition.to_owned(), |text, (label, _)| {
        text.replace(label, &format!(" {label} "))
    });
    for word in definition.split(|c: char| c.is_whitespace() || markers(c)) {
        let word = word.to_lowercase();
        let structural = word.is_empty()
            || word == headword
            || word.chars().all(|c| c.is_ascii_digit())
            || word.starts_with('.') && word[1..].chars().all(|c| c.is_ascii_digit());
        let word = word
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .trim_end_matches(|c: char| c.is_ascii_digit())
            .trim_end_matches('.');
        assert!(
            structural || shown.contains(word),
            "{term}: lost {word:?} from {shown:?}"
        );
    }
}

#[test]
fn an_english_entry_groups_numbered_senses_under_each_part_of_speech() {
    let detail = detail("run", ENGLISH);
    assert_eq!(
        detail.metadata,
        [Metadata {
            label: "Pronunciation".into(),
            value: "rʌn".into()
        }]
    );
    assert_eq!(
        outline(&detail.body.blocks),
        [
            "###Verb",
            "(runs, running; past ran; past participle run)",
            "1. [no object] move on foot faster than walking: _she ran to the station_ | _he ran the last few yards._",
            "  - (of a machine) operate: _the engine runs on diesel._",
            "  - [with object] cause to operate: _run the dishwasher twice a week._",
            "2. [with object] be in charge of: _he runs a small bakery_ | (-run as adjective) : _a family-run hotel._",
            "###Noun",
            "1. an act of running: _a morning run._",
            "2. a journey on a regular route: _the school run._",
            "##Phrases",
            "on the run 1 trying to avoid being caught. 2 while running: he caught it on the run. [late 18th century]",
            "##Origin",
            "Old English rinnan.",
        ]
    );
    assert_keeps_every_word("run", ENGLISH, "run");
}

#[test]
fn a_japanese_english_entry_lists_its_marked_examples() {
    let detail = detail("猫", JAPANESE_ENGLISH);
    assert!(detail.metadata.is_empty());
    assert_eq!(
        outline(&detail.body.blocks),
        [
            "ねこ 【猫】 a cat ; ｟愛称｠ a puss | pʊs |, a pussy (cat) (!主に小児語) .",
            "- _三毛猫 a tortoiseshell cat._",
            "- _黒猫 a black cat._",
            "- _猫を飼っている have [keep] a cat._",
        ]
    );
    assert_keeps_every_word("猫", JAPANESE_ENGLISH, "");
}

#[test]
fn an_english_japanese_entry_numbers_senses_that_follow_a_full_stop() {
    let detail = detail("happy", ENGLISH_JAPANESE);
    assert_eq!(detail.metadata[0].value, "hǽpi");
    assert_eq!(
        outline(&detail.body.blocks),
        [
            "形容詞-pier; -piest",
            "1. 〈人が〉うれしい, 楽しい (→ glad 2)",
            "  - _I'm happy to see you. 会えてうれしい._",
            "2. 〖通例名詞の前で〗幸せな",
            "  - _a happy ending 幸せな結末._",
        ]
    );
    assert_keeps_every_word("happy", ENGLISH_JAPANESE, "hap･py");
}

#[test]
fn a_japanese_entry_numbers_circled_senses_and_nests_katakana_sub_senses() {
    let detail = detail("走る", JAPANESE);
    assert_eq!(
        outline(&detail.body.blocks),
        [
            "はし・る 2【走る】（動ラ五）",
            "1. 足をはやく動かして進む。「廊下を―・る」",
            "2. 乗り物が進む。〈枕草子•12〉",
            "3. 液体が動く。",
            "  - ㋐ はやく流れる。",
            "  - ㋑ 飛びはねる。",
        ]
    );
    assert_keeps_every_word("走る", JAPANESE, "");
}

#[test]
fn a_thesaurus_entry_numbers_senses_and_labels_antonyms() {
    let detail = detail("happy", THESAURUS);
    assert!(detail.metadata.is_empty());
    assert_eq!(
        outline(&detail.body.blocks),
        [
            "###Adjective",
            "1. she looked happy and relaxed. cheerful, merry, joyful, glad; informal chirpy. **Antonyms** sad.",
            "2. we are happy to help. willing, glad, ready. **Antonyms** unwilling.",
            "##Word links",
            "hedonic: relating to pleasure",
        ]
    );
    assert_keeps_every_word("happy", THESAURUS, "happy");
}

#[test]
fn definitions_in_other_layouts_keep_all_their_text() {
    for definition in [
        "UN is short for United Nations.",
        "other | word | text",
        "Japanese 日本 | にほん | text with ORIGINAL words",
        "a word that names a thing; see verb. 2 is not a sense here",
    ] {
        let detail = detail("UN", definition);
        assert!(detail.metadata.is_empty(), "{definition}");
        assert_eq!(outline(&detail.body.blocks), [definition], "{definition}");
    }
}

#[test]
fn words_that_only_look_like_structure_stay_in_the_text() {
    let detail = detail(
        "cat",
        "cat | kat | noun 1 a small mammal kept as a pet; see big cat. a word, not a verb • a wild \
         cat: a marbled cat. • the cat in 〈伊勢物語•87〉 2 informal a man: this cat plays 2 horns.",
    );
    assert_eq!(
        outline(&detail.body.blocks),
        [
            "###Noun",
            "1. a small mammal kept as a pet; see big cat. a word, not a verb",
            "  - a wild cat: _a marbled cat._",
            "  - the cat in 〈伊勢物語•87〉",
            "2. informal a man: _this cat plays 2 horns._",
        ]
    );
}

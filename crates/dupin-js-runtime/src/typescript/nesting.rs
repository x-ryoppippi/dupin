//! How deeply a module's brackets nest, read without parsing it, so that a module nested too
//! deeply for a parser that recurses at every level is refused before one is spent on it.

/// Deeper than written code nests. Past it, each level costs oxc's parser up to a KiB of stack.
pub(super) const MAX_DEPTH: usize = 1_000;

#[derive(Clone, Copy, PartialEq)]
enum Open {
    Paren,
    Bracket,
    Brace,
    /// The `${` of a template, which its `}` closes back into the template's text.
    Interpolation,
    /// A `<` straight after a name, which opens type arguments or parameters as often as it
    /// compares. A `>`, `;` or a closing bracket ends it, so a comparison doesn't add up.
    Angle,
}

/// Where the opening bracket that nests deeper than [`MAX_DEPTH`] is, if one does. Strings,
/// comments, regular expressions and template text are skipped, so brackets inside them don't
/// count.
pub(super) fn too_deep(source: &[u8]) -> Option<usize> {
    let mut open: Vec<Open> = Vec::new();
    let mut at = 0;
    // Whether the last token ends an operand, so that a `/` after it divides and `<` compares.
    let mut operand = false;
    let mut name = false;
    while at < source.len() {
        let byte = source[at];
        let next = source.get(at + 1).copied();
        let mut ends_operand = false;
        let mut is_name = false;
        match byte {
            b'/' if next == Some(b'/') => {
                at = line_end(source, at);
                continue;
            }
            b'/' if next == Some(b'*') => {
                at = source[at + 2..]
                    .windows(2)
                    .position(|pair| pair == b"*/")
                    .map_or(source.len(), |found| at + 2 + found + 2);
                continue;
            }
            b' ' | b'\t' | b'\n' | b'\r' => {
                at += 1;
                continue;
            }
            b'\'' | b'"' => {
                at = string_end(source, at);
                operand = true;
                name = false;
                continue;
            }
            b'`' => {
                let (end, interpolation) = template_text(source, at + 1);
                at = end;
                if interpolation {
                    open.push(Open::Interpolation);
                    operand = false;
                } else {
                    operand = true;
                }
                name = false;
                if open.len() > MAX_DEPTH {
                    return Some(end - 2);
                }
                continue;
            }
            b'/' if !operand => {
                at = regex_end(source, at);
                operand = true;
                name = false;
                continue;
            }
            b'(' | b'[' | b'{' => open.push(match byte {
                b'(' => Open::Paren,
                b'[' => Open::Bracket,
                _ => Open::Brace,
            }),
            b'<' if name => open.push(Open::Angle),
            b')' | b']' | b'}' => {
                while open.last() == Some(&Open::Angle) {
                    open.pop();
                }
                let closes = match byte {
                    b')' => Open::Paren,
                    b']' => Open::Bracket,
                    _ if open.last() == Some(&Open::Interpolation) => Open::Interpolation,
                    _ => Open::Brace,
                };
                if open.last() == Some(&closes) {
                    open.pop();
                }
                if closes == Open::Interpolation {
                    let (end, interpolation) = template_text(source, at + 1);
                    at = end;
                    if interpolation {
                        open.push(Open::Interpolation);
                        operand = false;
                    } else {
                        operand = true;
                    }
                    name = false;
                    continue;
                }
                ends_operand = true;
            }
            b'>' => {
                if open.last() == Some(&Open::Angle) {
                    open.pop();
                }
            }
            // None of these is in a type that `<` opens, unless as the default of a type
            // parameter, which only ends the count early.
            b';' | b'=' | b'&' | b'|' if !(byte == b'=' && next == Some(b'>')) => {
                while open.last() == Some(&Open::Angle) {
                    open.pop();
                }
            }
            byte if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte >= 0x80 => {
                let length = source[at..]
                    .iter()
                    .take_while(|byte| {
                        byte.is_ascii_alphanumeric() || **byte == b'_' || **byte == b'$' || **byte >= 0x80
                    })
                    .count();
                let word = &source[at..at + length];
                ends_operand = !BEFORE_EXPRESSION.contains(&word);
                is_name = ends_operand && !byte.is_ascii_digit();
                at += length;
                operand = ends_operand;
                name = is_name;
                continue;
            }
            _ => {}
        }
        if open.len() > MAX_DEPTH {
            return Some(at);
        }
        operand = ends_operand;
        name = is_name;
        at += 1;
    }
    None
}

/// Words after which a `/` starts a regular expression rather than dividing.
const BEFORE_EXPRESSION: &[&[u8]] = &[
    b"return",
    b"typeof",
    b"instanceof",
    b"in",
    b"of",
    b"new",
    b"delete",
    b"void",
    b"throw",
    b"case",
    b"do",
    b"else",
    b"yield",
    b"await",
];

fn line_end(source: &[u8], from: usize) -> usize {
    source[from..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(source.len(), |found| from + found)
}

/// Just past the string that starts at `from`, or the line break that ends it unclosed.
fn string_end(source: &[u8], from: usize) -> usize {
    let quote = source[from];
    let mut at = from + 1;
    while at < source.len() {
        match source[at] {
            b'\\' => at += 2,
            b'\n' => return at,
            byte if byte == quote => return at + 1,
            _ => at += 1,
        }
    }
    source.len()
}

/// Just past the template text that starts at `from`, and whether a `${` rather than the
/// closing backtick ends it.
fn template_text(source: &[u8], from: usize) -> (usize, bool) {
    let mut at = from;
    while at < source.len() {
        match source[at] {
            b'\\' => at += 2,
            b'`' => return (at + 1, false),
            b'$' if source.get(at + 1) == Some(&b'{') => return (at + 2, true),
            _ => at += 1,
        }
    }
    (source.len(), false)
}

/// Just past the regular expression that starts at `from`.
fn regex_end(source: &[u8], from: usize) -> usize {
    let mut at = from + 1;
    let mut class = false;
    while at < source.len() {
        match source[at] {
            b'\\' => at += 2,
            b'\n' => return at,
            b'[' => {
                class = true;
                at += 1;
            }
            b']' => {
                class = false;
                at += 1;
            }
            b'/' if !class => return at + 1,
            _ => at += 1,
        }
    }
    source.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brackets_count_where_they_are_code() {
        let deep = MAX_DEPTH + 1;
        for (open, close) in [("(", ")"), ("[", "]"), ("{", "}"), ("`${", "}`"), ("A<", ">")] {
            let source = format!("let a = {}1{};", open.repeat(deep), close.repeat(deep));
            let found = too_deep(source.as_bytes()).unwrap_or_else(|| panic!("{open}"));
            let deepest = "let a = ".len() + MAX_DEPTH * open.len();
            assert!(
                (deepest..deepest + open.len()).contains(&found),
                "{open}: {found}"
            );
            let shallow = format!("let a = {}1{};", open.repeat(MAX_DEPTH), close.repeat(MAX_DEPTH));
            assert_eq!(too_deep(shallow.as_bytes()), None, "{open}");
        }
        let hidden = "(".repeat(deep);
        for source in [
            format!("const a = '{hidden}';"),
            format!("const a = \"{hidden}\";"),
            format!("const a = `{hidden}`;"),
            format!("// {hidden}\nconst a = 1;"),
            format!("/* {hidden} */ const a = 1;"),
            format!("const a = /[{hidden}]/;"),
            format!("const a = /\\({}/;", "\\(".repeat(deep)),
            format!("return /{hidden}/;"),
        ] {
            assert_eq!(too_deep(source.as_bytes()), None, "{source}");
        }
    }

    #[test]
    fn comparisons_dont_add_up() {
        let line = "if (i < n) x = a < b\nfor (let i = 0; i < n; i++) {}\nconst c = d < e ? f : g;\n";
        let source = format!("{{\n{}}}", line.repeat(MAX_DEPTH));
        assert_eq!(too_deep(source.as_bytes()), None);
        let divided = format!("const a = b / {}1{} / c;", "(".repeat(10), ")".repeat(10));
        assert_eq!(too_deep(divided.as_bytes()), None);
    }
}

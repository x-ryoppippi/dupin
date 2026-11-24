//! TypeScript that runs once its types are erased. Every type-only span is replaced by spaces of
//! the same length, keeping line breaks, so a line and column QuickJS reports in the result is
//! the same in the author's file and no source map is needed. Syntax that only a compiler can
//! turn into JavaScript, such as `enum`, is rejected instead. Nothing is type-checked.

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};

use crate::Diagnostic;

mod nesting;

const NEEDS_BUILD: &str = "typescript-needs-build";

/// Whether the package file `name` is TypeScript that Dupin erases, rather than a declaration
/// file, which only describes types.
pub(crate) fn is_typescript(name: &str) -> bool {
    let module = [".ts", ".mts", ".tsx"]
        .iter()
        .any(|suffix| name.ends_with(suffix));
    let declaration = [".d.ts", ".d.mts", ".d.tsx"]
        .iter()
        .any(|suffix| name.ends_with(suffix));
    module && !declaration
}

/// The stack an erasure needs besides its nesting.
const STACK_BASE_BYTES: usize = 256 * 1024;
/// Twice the most stack that oxc's parser and the eraser's walk took for a byte of source,
/// measured over every kind of nesting, so no file can overflow it. A level of nesting takes
/// at least a byte.
const STACK_BYTES_PER_SOURCE_BYTE: usize = if cfg!(debug_assertions) { 4096 } else { 2048 };

/// `source`, the package file `name`, as JavaScript with every TypeScript-only span blanked.
pub(crate) fn erase(name: &str, source: &str) -> Result<String, Diagnostic> {
    if let Some(offset) = nesting::too_deep(source.as_bytes()) {
        let (line, column) = position(source, offset as u32);
        return Err(Diagnostic::new(
            "stack-limit",
            format!(
                "nests too deeply to compile, past {} levels of brackets",
                nesting::MAX_DEPTH
            ),
        )
        .file(name)
        .at(line, column)
        .hint("Flatten deeply nested expressions, arrays, objects or types."));
    }
    // oxc's parser recurses once per level of nesting and sets no limit of its own. Brackets
    // are limited above, but a chain such as `!!!…1` or `a = a = …` nests without them, so the
    // stack is sized to the source, which no nesting can overflow.
    let stack = source
        .len()
        .saturating_mul(STACK_BYTES_PER_SOURCE_BYTE)
        .saturating_add(STACK_BASE_BYTES);
    std::thread::scope(|scope| {
        let erasing = std::thread::Builder::new()
            .name("typescript".into())
            .stack_size(stack)
            .spawn_scoped(scope, || erase_on_this_stack(name, source));
        match erasing {
            Ok(erasing) => erasing
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            Err(error) => Err(Diagnostic::new(
                "engine-error",
                format!("Couldn't start erasing the types: {error}"),
            )
            .file(name)),
        }
    })
}

fn erase_on_this_stack(name: &str, source: &str) -> Result<String, Diagnostic> {
    let source_type = if name.ends_with(".tsx") {
        SourceType::tsx()
    } else {
        SourceType::ts()
    }
    .with_module(true);
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if let Some(error) = parsed.diagnostics.errors().next() {
        let offset = error.labels.first().map_or(0, |label| label.offset());
        let (line, column) = position(source, offset);
        return Err(Diagnostic::new("syntax-error", error.message.to_string())
            .file(name)
            .at(line, column)
            .hint(
                "Fix the syntax. Dupin runs TypeScript by erasing its types; build anything else with `dupin ext build`.",
            ));
    }
    let mut eraser = Eraser {
        text: source.as_bytes(),
        blank: Vec::new(),
        put: Vec::new(),
        problem: None,
    };
    eraser.visit_program(&parsed.program);
    if let Some((span, construct)) = eraser.problem {
        let (line, column) = position(source, span.start);
        return Err(Diagnostic::new(NEEDS_BUILD, construct.message())
            .file(name)
            .at(line, column)
            .hint(construct.hint()));
    }
    Ok(eraser.render())
}

/// The 1-based line and byte column of `offset`, counted as QuickJS counts them.
fn position(source: &str, offset: u32) -> (u32, u32) {
    let before = &source.as_bytes()[..(offset as usize).min(source.len())];
    let line = before.iter().filter(|&&byte| byte == b'\n').count() + 1;
    let line_start = before
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |newline| newline + 1);
    (line as u32, (before.len() - line_start + 1) as u32)
}

/// TypeScript that has no JavaScript without a compiler's transform.
#[derive(Clone, Copy, Debug)]
enum Construct {
    Enum,
    Namespace,
    ParameterProperty,
    Decorator,
    Jsx,
    ImportEquals,
    ExportAssignment,
    TypeAssertion,
}

impl Construct {
    fn message(self) -> &'static str {
        match self {
            Self::Enum => "declares an `enum`, which only a TypeScript build turns into JavaScript",
            Self::Namespace => {
                "declares a `namespace` holding values, which only a TypeScript build turns into JavaScript"
            }
            Self::ParameterProperty => {
                "declares a parameter property, which only a TypeScript build turns into a field assignment"
            }
            Self::Decorator => "uses a decorator, which Dupin's JavaScript engine can't run without a build",
            Self::Jsx => "uses JSX, which only a build turns into JavaScript",
            Self::ImportEquals => "uses `import … =`, which only a TypeScript build turns into JavaScript",
            Self::ExportAssignment => "uses `export =`, which only a TypeScript build turns into JavaScript",
            Self::TypeAssertion => {
                "uses a `<T>value` assertion, whose erased type can end a `return` before its value"
            }
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Enum => "Use an object with `as const` instead, or build it with `dupin ext build`.",
            Self::Namespace => "Export the values from a module instead, or build it with `dupin ext build`.",
            Self::ParameterProperty => {
                "Declare the field in the class body and assign it in the constructor, or build it with `dupin ext build`."
            }
            Self::Decorator => {
                "Call the decorator's function directly instead, or build it with `dupin ext build`."
            }
            Self::Jsx => "Build it with `dupin ext build`.",
            Self::ImportEquals => "Use `import … from` instead, or build it with `dupin ext build`.",
            Self::ExportAssignment => "Use `export default` instead, or build it with `dupin ext build`.",
            Self::TypeAssertion => "Write `value as T` instead, or build it with `dupin ext build`.",
        }
    }
}

enum Outcome {
    /// JavaScript, possibly with type-only parts inside it.
    Keep,
    /// Types alone, erased whole.
    Erase,
    Reject(Span, Construct),
}

fn statement(statement: &Statement<'_>) -> Outcome {
    match statement {
        Statement::ImportDeclaration(import) if import.import_kind.is_type() => Outcome::Erase,
        Statement::ExportNamedDeclaration(export) if export.export_kind.is_type() => Outcome::Erase,
        Statement::ExportFromDeclaration(export) if export.export_kind.is_type() => Outcome::Erase,
        Statement::ExportAllDeclaration(export) if export.export_kind.is_type() => Outcome::Erase,
        Statement::ExportDeclaration(export) => declaration(&export.declaration),
        Statement::ExportDefaultDeclaration(export) => match &export.declaration {
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => Outcome::Erase,
            ExportDefaultDeclarationKind::FunctionDeclaration(function) if function.body.is_none() => {
                Outcome::Erase
            }
            _ => Outcome::Keep,
        },
        Statement::TSExportAssignment(export) => Outcome::Reject(export.span, Construct::ExportAssignment),
        Statement::TSNamespaceExportDeclaration(_) => Outcome::Erase,
        _ => match statement.as_declaration() {
            Some(found) => declaration(found),
            None => Outcome::Keep,
        },
    }
}

fn declaration(declaration: &Declaration<'_>) -> Outcome {
    match declaration {
        Declaration::VariableDeclaration(variables) if variables.declare => Outcome::Erase,
        // A function without a body is an overload signature or a `declare`.
        Declaration::FunctionDeclaration(function) if function.declare || function.body.is_none() => {
            Outcome::Erase
        }
        Declaration::ClassDeclaration(class) if class.declare => Outcome::Erase,
        Declaration::TSTypeAliasDeclaration(_)
        | Declaration::TSInterfaceDeclaration(_)
        | Declaration::TSGlobalDeclaration(_) => Outcome::Erase,
        Declaration::TSEnumDeclaration(declared) if declared.declare => Outcome::Erase,
        Declaration::TSEnumDeclaration(declared) => Outcome::Reject(declared.span, Construct::Enum),
        Declaration::TSNamespaceDeclaration(namespace) => {
            if namespace.declare || namespace_is_types(namespace) {
                Outcome::Erase
            } else {
                Outcome::Reject(namespace.span, Construct::Namespace)
            }
        }
        Declaration::TSExternalModuleDeclaration(module) => {
            let types = module.body.as_ref().is_none_or(|body| block_is_types(&body.body));
            if module.declare || types {
                Outcome::Erase
            } else {
                Outcome::Reject(module.span, Construct::Namespace)
            }
        }
        Declaration::TSImportEqualsDeclaration(import) if import.import_kind.is_type() => Outcome::Erase,
        Declaration::TSImportEqualsDeclaration(import) => {
            Outcome::Reject(import.span, Construct::ImportEquals)
        }
        _ => Outcome::Keep,
    }
}

/// TypeScript compiles a namespace to an object only when something in it has a value.
fn namespace_is_types(namespace: &TSNamespaceDeclaration<'_>) -> bool {
    match &namespace.body {
        TSNamespaceDeclarationBody::TSNamespaceDeclaration(inner) => {
            inner.declare || namespace_is_types(inner)
        }
        TSNamespaceDeclarationBody::TSModuleBlock(block) => block_is_types(&block.body),
    }
}

fn block_is_types(statements: &[Statement<'_>]) -> bool {
    statements
        .iter()
        .all(|found| matches!(statement(found), Outcome::Erase))
}

/// Class member modifiers that only TypeScript reads.
const MODIFIERS: &[&[u8]] = &[
    b"public",
    b"private",
    b"protected",
    b"readonly",
    b"override",
    b"declare",
    b"abstract",
];

struct Eraser<'s> {
    text: &'s [u8],
    blank: Vec<(u32, u32)>,
    /// Bytes written over a blanked range, such as the `;` that ends an erased statement.
    put: Vec<(u32, u8)>,
    problem: Option<(Span, Construct)>,
}

impl Eraser<'_> {
    fn blank(&mut self, start: u32, end: u32) {
        if start < end {
            self.blank.push((start, end));
        }
    }

    fn blank_span(&mut self, span: Span) {
        self.blank(span.start, span.end);
    }

    /// Erases a whole statement or class member. The `;` left in its place keeps the next one
    /// from joining the one before, as `a\n(b)` would.
    fn erase(&mut self, span: Span) {
        self.blank_span(span);
        self.put.push((span.start, b';'));
    }

    fn reject(&mut self, span: Span, construct: Construct) {
        self.problem.get_or_insert((span, construct));
    }

    /// The first byte at or after `from` that is neither whitespace nor in a comment, and whether
    /// a line break comes before it.
    fn next_token(&self, mut from: usize) -> (usize, bool) {
        let text = self.text;
        let mut newline = false;
        while from < text.len() {
            match text[from] {
                b'\n' | b'\r' => {
                    newline = true;
                    from += 1;
                }
                b' ' | b'\t' | 0x0b | 0x0c => from += 1,
                b'/' if text.get(from + 1) == Some(&b'/') => {
                    while from < text.len() && text[from] != b'\n' {
                        from += 1;
                    }
                }
                b'/' if text.get(from + 1) == Some(&b'*') => {
                    let end = text[from + 2..]
                        .windows(2)
                        .position(|pair| pair == b"*/")
                        .map_or(text.len(), |found| from + 2 + found + 2);
                    newline |= text[from..end].iter().any(|&byte| byte == b'\n' || byte == b'\r');
                    from = end;
                }
                byte if byte >= 0x80 => {
                    let rest = std::str::from_utf8(&text[from..]).ok();
                    match rest.and_then(|rest| rest.chars().next()) {
                        Some(ch) if ch.is_whitespace() || ch == '\u{feff}' => {
                            newline |= matches!(ch, '\u{2028}' | '\u{2029}');
                            from += ch.len_utf8();
                        }
                        _ => break,
                    }
                }
                _ => break,
            }
        }
        (from, newline)
    }

    /// Where the `byte` token is in `start..end`, outside comments.
    fn find(&self, start: u32, end: u32, byte: u8) -> Option<u32> {
        let mut at = start as usize;
        while at < end as usize {
            let (next, _) = self.next_token(at);
            if next >= end as usize {
                return None;
            }
            if self.text[next] == byte {
                return Some(next as u32);
            }
            at = next + 1;
        }
        None
    }

    /// Blanks the `,` after an erased list item, if one follows it.
    fn blank_comma_after(&mut self, end: u32) {
        let (next, _) = self.next_token(end as usize);
        if self.text.get(next) == Some(&b',') {
            self.blank(next as u32, next as u32 + 1);
        }
    }

    /// The words in `start..end` outside comments, as spans. Only for a stretch of keywords
    /// and names, which holds no string to look inside.
    fn words(&self, start: u32, end: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        let end = end as usize;
        let mut at = start as usize;
        std::iter::from_fn(move || {
            loop {
                let (next, _) = self.next_token(at);
                if next >= end {
                    return None;
                }
                let length = self.text[next..end]
                    .iter()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_' || **byte == b'$')
                    .count();
                at = next + length.max(1);
                if length > 0 {
                    return Some((next as u32, (next + length) as u32));
                }
            }
        })
    }

    /// Where the keyword `word` starts in `start..end`.
    fn find_word(&self, start: u32, end: u32, word: &[u8]) -> Option<u32> {
        self.words(start, end)
            .find(|&(from, to)| &self.text[from as usize..to as usize] == word)
            .map(|(from, _)| from)
    }

    /// Blanks TypeScript's modifiers among the words in `start..end`, the part of a class
    /// member before its name.
    fn blank_modifiers(&mut self, start: u32, end: u32) {
        let modifiers: Vec<_> = self
            .words(start, end)
            .filter(|&(from, to)| MODIFIERS.contains(&&self.text[from as usize..to as usize]))
            .collect();
        for (from, to) in modifiers {
            self.blank(from, to);
        }
    }

    fn has_line_break(&self, start: u32, end: u32) -> bool {
        let text = &self.text[start as usize..end as usize];
        text.iter().any(|&byte| byte == b'\n' || byte == b'\r')
            || text
                .windows(3)
                .any(|character| character == "\u{2028}".as_bytes() || character == "\u{2029}".as_bytes())
    }

    /// Blanks the `?` or `!` TypeScript writes after a name, in `start..end`.
    fn blank_marker(&mut self, start: u32, end: u32, marker: u8) {
        if let Some(at) = self.find(start, end, marker) {
            self.blank(at, at + 1);
        }
    }

    /// Erases `as T` or `satisfies T` after `expression`, ending at `end`.
    fn erase_suffix(&mut self, expression: &Expression<'_>, end: u32) {
        self.blank(expression.span().end, end);
        // In TypeScript, `x as T` ends its statement when the next line starts with one of
        // these; once the type is gone, JavaScript would carry on into a call, index or tag.
        let (next, newline) = self.next_token(end as usize);
        if newline && matches!(self.text.get(next), Some(b'(' | b'[' | b'`')) {
            self.put.push((end - 1, b';'));
        }
    }

    fn render(self) -> String {
        let mut blanked = vec![false; self.text.len()];
        for (start, end) in self.blank {
            blanked[start as usize..end as usize].fill(true);
        }
        let mut output = self.text.to_vec();
        let mut at = 0;
        while at < output.len() {
            let width = utf8_width(output[at]);
            if blanked[at] {
                let character = &self.text[at..at + width];
                let is_line_break = matches!(character, b"\n" | b"\r")
                    || character == "\u{2028}".as_bytes()
                    || character == "\u{2029}".as_bytes();
                if !is_line_break {
                    output[at..at + width].fill(b' ');
                }
            }
            at += width;
        }
        for (at, byte) in self.put {
            output[at as usize] = byte;
        }
        String::from_utf8(output).expect("only whole characters are replaced, by ASCII")
    }
}

fn utf8_width(first: u8) -> usize {
    match first {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}

impl<'a> Visit<'a> for Eraser<'_> {
    fn visit_statement(&mut self, it: &Statement<'a>) {
        if self.problem.is_some() {
            return;
        }
        match statement(it) {
            Outcome::Keep => walk::walk_statement(self, it),
            Outcome::Erase => {
                // A build inlines a `declare const enum`'s members where they are used, which
                // erasing it would leave naming nothing.
                let mut found = ConstEnum(None);
                found.visit_statement(it);
                match found.0 {
                    Some(span) => self.reject(span, Construct::Enum),
                    None => self.erase(it.span()),
                }
            }
            Outcome::Reject(span, construct) => self.reject(span, construct),
        }
    }

    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        for specifier in it.specifiers.iter().flatten() {
            if let ImportDeclarationSpecifier::ImportSpecifier(specifier) = specifier
                && specifier.import_kind.is_type()
            {
                self.blank_span(specifier.span);
                self.blank_comma_after(specifier.span.end);
            }
        }
    }

    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        self.blank_type_exports(&it.specifiers);
    }

    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        self.blank_type_exports(&it.specifiers);
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        let class = self.find_word(it.span.start, it.body.span.start, b"class");
        if it.r#abstract
            && let Some(class) = class
        {
            self.blank_modifiers(it.span.start, class);
        }
        if let (Some(first), Some(last)) = (it.implements.first(), it.implements.last()) {
            let heritage = it.heritage.as_ref().map(|heritage| {
                heritage
                    .type_arguments
                    .as_ref()
                    .map_or(heritage.expression.span().end, |arguments| arguments.span.end)
            });
            let after = [
                class.map(|class| class + 5),
                it.id.as_ref().map(|id| id.span.end),
                it.type_parameters.as_ref().map(|parameters| parameters.span.end),
                heritage,
            ]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or(it.span.start);
            if let Some(implements) = self.find_word(after, first.span.start, b"implements") {
                self.blank(implements, last.span.end);
            }
        }
        walk::walk_class(self, it);
    }

    fn visit_class_element(&mut self, it: &ClassElement<'a>) {
        let erased = match it {
            ClassElement::TSIndexSignature(signature) => Some(signature.span),
            ClassElement::MethodDefinition(method)
                if method.r#type.is_abstract() || method.value.body.is_none() =>
            {
                Some(method.span)
            }
            ClassElement::PropertyDefinition(property)
                if property.r#type.is_abstract() || property.declare =>
            {
                Some(property.span)
            }
            ClassElement::AccessorProperty(accessor) if accessor.r#type.is_abstract() => Some(accessor.span),
            _ => None,
        };
        match erased {
            Some(span) => self.erase(span),
            None => walk::walk_class_element(self, it),
        }
    }

    fn visit_method_definition(&mut self, it: &MethodDefinition<'a>) {
        self.blank_modifiers(it.span.start, it.key.span().start);
        if it.optional {
            self.blank_marker(it.key.span().end, it.value.params.span.start, b'?');
        }
        walk::walk_method_definition(self, it);
    }

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        self.blank_modifiers(it.span.start, it.key.span().start);
        let end = it
            .type_annotation
            .as_ref()
            .map(|annotation| annotation.span.start)
            .or_else(|| it.value.as_ref().map(|value| value.span().start))
            .unwrap_or(it.span.end);
        if it.optional {
            self.blank_marker(it.key.span().end, end, b'?');
        }
        if it.definite {
            self.blank_marker(it.key.span().end, end, b'!');
        }
        walk::walk_property_definition(self, it);
    }

    fn visit_accessor_property(&mut self, it: &AccessorProperty<'a>) {
        self.blank_modifiers(it.span.start, it.key.span().start);
        if it.definite
            && let Some(annotation) = &it.type_annotation
        {
            self.blank_marker(it.key.span().end, annotation.span.start, b'!');
        }
        walk::walk_accessor_property(self, it);
    }

    fn visit_formal_parameter(&mut self, it: &FormalParameter<'a>) {
        if it.accessibility.is_some() || it.readonly || it.r#override {
            return self.reject(it.span, Construct::ParameterProperty);
        }
        if it.optional {
            let end = it
                .type_annotation
                .as_ref()
                .map_or(it.span.end, |annotation| annotation.span.start);
            self.blank_marker(it.pattern.span().end, end, b'?');
        }
        walk::walk_formal_parameter(self, it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if it.definite
            && let Some(annotation) = &it.type_annotation
        {
            self.blank_marker(it.id.span().end, annotation.span.start, b'!');
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_ts_this_parameter(&mut self, it: &TSThisParameter<'a>) {
        self.blank_span(it.span);
        self.blank_comma_after(it.span.end);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        // A line break left where multi-line type parameters were would end `return <\n T,\n>(a)
        // => a` at `return`, or part `async` from its parameters, so the `(` moves to the `<`.
        if let Some(parameters) = &it.type_parameters {
            let open = it.params.span.start;
            if self.text[open as usize] == b'(' && self.has_line_break(parameters.span.start, open) {
                self.blank(open, open + 1);
                self.put.push((parameters.span.start, b'('));
            }
        }
        // JavaScript allows no line break between an arrow's parameters and `=>`, which a
        // multi-line return type leaves once blanked, so the `)` moves to the type's last byte.
        if let Some(annotation) = &it.return_type {
            let close = it.params.span.end - 1;
            if self.text[close as usize] == b')' && self.has_line_break(close, annotation.span.end) {
                self.blank(close, close + 1);
                self.put.push((annotation.span.end - 1, b')'));
            }
        }
        walk::walk_arrow_function_expression(self, it);
    }

    fn visit_ts_type_annotation(&mut self, it: &TSTypeAnnotation<'a>) {
        self.blank_span(it.span);
    }

    fn visit_ts_type_parameter_declaration(&mut self, it: &TSTypeParameterDeclaration<'a>) {
        self.blank_span(it.span);
    }

    fn visit_ts_type_parameter_instantiation(&mut self, it: &TSTypeParameterInstantiation<'a>) {
        self.blank_span(it.span);
    }

    fn visit_ts_class_implements(&mut self, _it: &TSClassImplements<'a>) {}

    fn visit_ts_as_expression(&mut self, it: &TSAsExpression<'a>) {
        self.erase_suffix(&it.expression, it.span.end);
        self.visit_expression(&it.expression);
    }

    fn visit_ts_satisfies_expression(&mut self, it: &TSSatisfiesExpression<'a>) {
        self.erase_suffix(&it.expression, it.span.end);
        self.visit_expression(&it.expression);
    }

    fn visit_ts_non_null_expression(&mut self, it: &TSNonNullExpression<'a>) {
        self.blank(it.expression.span().end, it.span.end);
        self.visit_expression(&it.expression);
    }

    fn visit_ts_type_assertion(&mut self, it: &TSTypeAssertion<'a>) {
        self.reject(it.span, Construct::TypeAssertion);
    }

    fn visit_decorator(&mut self, it: &Decorator<'a>) {
        self.reject(it.span, Construct::Decorator);
    }

    fn visit_jsx_element(&mut self, it: &JSXElement<'a>) {
        self.reject(it.span, Construct::Jsx);
    }

    fn visit_jsx_fragment(&mut self, it: &JSXFragment<'a>) {
        self.reject(it.span, Construct::Jsx);
    }
}

struct ConstEnum(Option<Span>);

impl<'a> Visit<'a> for ConstEnum {
    fn visit_ts_enum_declaration(&mut self, it: &TSEnumDeclaration<'a>) {
        if it.r#const {
            self.0.get_or_insert(it.span);
        }
    }
}

impl Eraser<'_> {
    fn blank_type_exports(&mut self, specifiers: &[ExportSpecifier<'_>]) {
        for specifier in specifiers {
            if specifier.export_kind.is_type() {
                self.blank_span(specifier.span);
                self.blank_comma_after(specifier.span.end);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ERASED: &[(&str, &str)] = &[
        ("let a: number = 1;", "let a         = 1;"),
        (
            "function f<T>(a: T): T { return a; }",
            "function f   (a   )    { return a; }",
        ),
        ("const g = <T,>(a: T): T => a;", "const g =     (a   )    => a;"),
        (
            "const h = (a: number): {\n  b: number;\n} => ({ b: a });",
            "const h = (a            \n            \n) => ({ b: a });",
        ),
        (
            "function i(this: Window, a: number) {}",
            "function i(              a        ) {}",
        ),
        (
            "function j(a?: number, ...rest: string[]) {}",
            "function j(a         , ...rest          ) {}",
        ),
        (
            "interface K { a: number }\n(k)();",
            ";                        \n(k)();",
        ),
        (
            "type L = string;\n[l].map(String);",
            ";               \n[l].map(String);",
        ),
        (
            "export interface M {}\nexport type N = 1;\nexport default interface O {}",
            ";                    \n;                 \n;                            ",
        ),
        (
            "import type { P } from './p.ts';\nimport type Q from './q.ts';",
            ";                               \n;                           ",
        ),
        (
            "import R, { type S, t, type U as V } from './r.ts';",
            "import R, {         t,             } from './r.ts';",
        ),
        (
            "export { type S, t };\nexport type { P };\nexport { type W } from './w.ts';\nexport type * from './x.ts';",
            "export {         t };\n;                 \nexport {        } from './w.ts';\n;                           ",
        ),
        (
            "const y = z as unknown as string;",
            "const y = z                     ;",
        ),
        ("const y = z satisfies Z;", "const y = z            ;"),
        ("const y = z!.a!;", "const y = z .a ;"),
        // Each `(` moves to its `<`: a line break after `return`, `yield` or `async` would
        // end the statement there or make `async(a)` a call.
        (
            "function f() {\n  return <\n    T,\n  >(a: T) => a;\n}",
            "function f() {\n  return (\n      \n    a   ) => a;\n}",
        ),
        (
            "function* g() {\n  yield <\n    T,\n  >(a: T) => a;\n}",
            "function* g() {\n  yield (\n      \n    a   ) => a;\n}",
        ),
        (
            "const f = async <\n  T,\n>(a: T) => a;",
            "const f = async (\n    \n  a   ) => a;",
        ),
        (
            "interface I {}\nclass A implements /* implements */ I {}\nclass B /* implements */ implements I, J {}",
            ";             \nclass A                               {}\nclass B /* implements */                 {}",
        ),
        (
            "abstract /* class */ class C {}",
            "         /* class */ class C {}",
        ),
        // TypeScript ends the statement after the type, as the `;` does once it is gone.
        ("const y = z as T\n(a);", "const y = z    ;\n(a);"),
        (
            "f<string>(1);\nnew G<number>();\nconst h = f<string>;\ntag<T>`x`;",
            "f        (1);\nnew G        ();\nconst h = f        ;\ntag   `x`;",
        ),
        (
            "declare const a: number;\ndeclare function b(): void;\ndeclare class C {}\ndeclare enum D { E }\ndeclare namespace F { const g: 1 }\ndeclare module 'h' {}\ndeclare global { interface Window {} }",
            ";                       \n;                          \n;                 \n;                   \n;                                 \n;                    \n;                                     ",
        ),
        (
            "namespace Types { export type A = 1; export interface B {} }",
            ";                                                           ",
        ),
        (
            "function o(a: string): string;\nfunction o(a: number): number;\nfunction o(a: any) { return a; }\nexport function p(): void;\nexport function p() {}\nexport default function q(): void;",
            ";                             \n;                             \nfunction o(a     ) { return a; }\n;                         \nexport function p() {}\n;                                 ",
        ),
        (
            "abstract class A<T> extends B<T> implements C, D<T> {\n  private readonly x: number = 1;\n  declare y: string;\n  z?: number;\n  w!: number;\n  static [key: string]: unknown;\n  abstract m(): void;\n  public static override n<U>(a: U): U { return a; }\n  protected o?(): void {}\n  constructor(a: string);\n  constructor(a: unknown) { super(); }\n}",
            "         class A    extends B                       {\n                   x         = 1;\n  ;                 \n  z         ;\n  w         ;\n  ;                             \n  ;                  \n         static          n   (a   )    { return a; }\n            o ()       {}\n  ;                      \n  constructor(a         ) { super(); }\n}",
        ),
        ("export abstract class E {}", "export          class E {}"),
        ("let q!: number;", "let q         ;"),
        (
            "try {} catch (error: unknown) {}",
            "try {} catch (error         ) {}",
        ),
        ("const é: number = 1; // é", "const é         = 1; // é"),
    ];

    #[test]
    fn erasable_syntax_leaves_javascript_at_the_same_positions() {
        for (typescript, javascript) in ERASED {
            let erased = erase("main.ts", typescript).unwrap_or_else(|error| panic!("{typescript}: {error}"));
            assert_eq!(erased, *javascript, "{typescript}");
            let breaks = |text: &str| -> Vec<usize> { text.match_indices('\n').map(|(at, _)| at).collect() };
            assert_eq!(erased.len(), typescript.len(), "{typescript}");
            assert_eq!(breaks(&erased), breaks(typescript), "{typescript}");
            let allocator = Allocator::default();
            let parsed = Parser::new(&allocator, &erased, SourceType::mjs()).parse();
            assert!(
                parsed.diagnostics.errors().next().is_none(),
                "{erased:?} is not JavaScript: {:?}",
                parsed.diagnostics
            );
        }
    }

    #[test]
    fn syntax_that_needs_a_transform_is_rejected_where_it_is() {
        let cases = [
            ("let a = 1;\nenum E { A }", 2, 1, "`enum`"),
            ("const enum E { A }", 1, 1, "`enum`"),
            ("namespace N { export const x = 1; }", 1, 1, "`namespace`"),
            ("module M { export function f() {} }", 1, 1, "`namespace`"),
            (
                "class P {\n  constructor(private x: number) {}\n}",
                2,
                15,
                "parameter property",
            ),
            (
                "class P { constructor(readonly x: number) {} }",
                1,
                23,
                "parameter property",
            ),
            ("class Q {\n  @dec method() {}\n}", 2, 3, "decorator"),
            ("@dec\nclass R {}", 1, 1, "decorator"),
            ("import fs = require('fs');", 1, 1, "`import … =`"),
            ("import A = B.C;", 1, 1, "`import … =`"),
            ("export = 1;", 1, 1, "`export =`"),
            (
                "declare const enum E { A = 1 }\nconsole.log(E.A);",
                1,
                1,
                "`enum`",
            ),
            ("declare namespace N { const enum E { A } }", 1, 23, "`enum`"),
            ("const y = <string>z;", 1, 11, "`<T>value`"),
            (
                "function f(o: unknown) {\n  return <{\n    a: number\n  }>o;\n}",
                2,
                10,
                "`<T>value`",
            ),
        ];
        for (source, line, column, construct) in cases {
            let diagnostic = erase("lib/util.ts", source).expect_err(source);
            assert_eq!(diagnostic.code, NEEDS_BUILD, "{source}");
            assert_eq!(diagnostic.file.as_deref(), Some("lib/util.ts"), "{source}");
            assert_eq!(
                (diagnostic.line, diagnostic.column),
                (Some(line), Some(column)),
                "{source}"
            );
            assert!(
                diagnostic.message.contains(construct),
                "{source}: {}",
                diagnostic.message
            );
            assert!(
                diagnostic
                    .hint
                    .as_deref()
                    .is_some_and(|hint| hint.contains("dupin ext build")),
                "{source}"
            );
        }
    }

    #[test]
    fn jsx_is_rejected_and_tsx_without_it_is_erased() {
        let diagnostic = erase(
            "view.tsx",
            "const a: number = 1;\nexport const b = <div>{a}</div>;",
        )
        .unwrap_err();
        assert_eq!(diagnostic.code, NEEDS_BUILD);
        assert_eq!((diagnostic.line, diagnostic.column), (Some(2), Some(18)));
        assert!(diagnostic.message.contains("JSX"), "{}", diagnostic.message);
        assert_eq!(
            erase("view.tsx", "export const f = <T,>(a: T) => a;").unwrap(),
            "export const f =     (a   ) => a;"
        );
    }

    #[test]
    fn a_syntax_error_reports_where_the_parser_stopped() {
        let diagnostic = erase("main.ts", "let a = 1;\nlet b: = 2;").unwrap_err();
        assert_eq!(diagnostic.code, "syntax-error");
        assert_eq!(diagnostic.file.as_deref(), Some("main.ts"));
        assert_eq!(diagnostic.line, Some(2));
    }

    #[test]
    fn declaration_files_are_not_modules() {
        for name in ["main.ts", "lib/util.mts", "view.tsx"] {
            assert!(is_typescript(name), "{name}");
        }
        for name in [
            "dupin.d.ts",
            "types/x.d.mts",
            "main.js",
            "main.mjs",
            "main.cts",
            "a.ts.js",
        ] {
            assert!(!is_typescript(name), "{name}");
        }
    }

    fn nested(construct: &str, depth: usize) -> String {
        match construct {
            "[" => format!("const a = {}1{};", "[".repeat(depth), "]".repeat(depth)),
            "(" => format!("const a = {}1{};", "(".repeat(depth), ")".repeat(depth)),
            "{" => format!("{}{}", "{".repeat(depth), "}".repeat(depth)),
            "!" => format!("const a = {}1;", "!".repeat(depth)),
            "=" => format!("let a; {}1;", "a=".repeat(depth)),
            "=>" => format!("const f = {}1;", "a=>".repeat(depth)),
            "?" => format!("const a = {}1;", "a?1:".repeat(depth)),
            "if" => format!("{};", "if(a)".repeat(depth)),
            "${" => format!("const a = {}1{};", "`${".repeat(depth), "}`".repeat(depth)),
            "new" => format!("const a = {}A;", "new ".repeat(depth)),
            "A<" => format!("let a: {}B{};", "A<".repeat(depth), ">".repeat(depth)),
            "[]" => format!("let a: {}B{};", "[".repeat(depth), "]".repeat(depth)),
            "()" => format!("let a: {}B{};", "(".repeat(depth), ")".repeat(depth)),
            "extends" => format!("let a: {}B;", "A extends B ? C : ".repeat(depth)),
            _ => unreachable!("{construct}"),
        }
    }

    #[test]
    fn nesting_deeper_than_a_thread_stack_holds_is_refused_or_erased() {
        for construct in ["[", "(", "{", "${", "A<", "[]", "()"] {
            let source = nested(construct, 100_000);
            let diagnostic = erase("main.ts", &source).expect_err(construct);
            assert_eq!(diagnostic.code, "stack-limit", "{construct}");
            assert_eq!(diagnostic.line, Some(1), "{construct}");
            assert!(
                diagnostic.column.is_some_and(|column| column > 1_000),
                "{construct}"
            );
        }
        for construct in ["!", "=", "=>", "?", "if", "new", "extends"] {
            let source = nested(construct, 100_000);
            let erased = erase("main.ts", &source).unwrap_or_else(|error| panic!("{construct}: {error}"));
            assert_eq!(erased.len(), source.len(), "{construct}");
        }
    }

    /// `cargo test --release -p dupin-js-runtime -- --ignored --nocapture typescript::tests::time`
    #[test]
    #[ignore = "a measurement, not a check"]
    fn time_erasing_2000_lines() {
        let block = "interface Item { title: string; subtitle?: string }\n\
            type Score = { item: Item; score: number };\n\
            export function rank<T extends Item>(items: readonly T[], query: string): Score[] {\n\
            \tconst scores: Score[] = [];\n\
            \tfor (const item of items) {\n\
            \t\tconst score = (item.title as string).indexOf(query)!;\n\
            \t\tif (score >= 0) scores.push({ item, score } satisfies Score);\n\
            \t}\n\
            \treturn scores.sort((a: Score, b: Score): number => a.score - b.score);\n\
            }\n\
            class Cache<K, V> implements Iterable<V> {\n\
            \tprivate readonly entries = new Map<K, V>();\n\
            \tdeclare size: number;\n\
            \tget(key: K): V | undefined { return this.entries.get(key); }\n\
            \tset(key: K, value: V): this { this.entries.set(key, value); return this; }\n\
            \t*[Symbol.iterator](): Iterator<V> { yield* this.entries.values(); }\n\
            }\n\
            export const limit: number = 50 as const;\n\
            let cache!: Cache<string, Score[]>;\n\
            export type { Item, Score };\n";
        let source = block.repeat(100);
        assert_eq!(source.lines().count(), 2000);
        erase("main.ts", &source).unwrap();
        let runs = 50;
        let mut times: Vec<_> = (0..runs)
            .map(|_| {
                let start = std::time::Instant::now();
                std::hint::black_box(erase("main.ts", std::hint::black_box(&source)).unwrap());
                start.elapsed()
            })
            .collect();
        times.sort();
        println!(
            "erasing {} lines ({} bytes): p50 {:?}, p95 {:?}, max {:?}",
            source.lines().count(),
            source.len(),
            times[runs / 2],
            times[runs * 95 / 100],
            times[runs - 1]
        );
    }
}

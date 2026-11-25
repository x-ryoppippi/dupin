use std::collections::{BTreeMap, BTreeSet};

use oxc_allocator::Allocator;
use oxc_ast::ast::{Declaration, PropertyKey, Statement, TSSignature, TSType};
use oxc_parser::Parser;
use oxc_span::SourceType;
use rquickjs::{Context, Module, Object, Runtime, Value};

use crate::{HOST_MODULE, host};

const DECLARATIONS: &str = include_str!("../../../../sdk/dupin.d.ts");

/// Each export's name, with the methods of an object export.
type Exports = BTreeMap<String, BTreeSet<String>>;

/// The value exports `declare module 'dupin'` declares, leaving out types.
fn declared() -> Exports {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, DECLARATIONS, SourceType::d_ts()).parse();
    assert!(parsed.diagnostics.errors().next().is_none(), "dupin.d.ts parses");
    let block = parsed
        .program
        .body
        .iter()
        .find_map(|statement| match statement {
            Statement::TSExternalModuleDeclaration(module) if module.id.value == HOST_MODULE => {
                module.body.as_deref()
            }
            _ => None,
        })
        .expect("dupin.d.ts declares module 'dupin'");
    let mut exports = Exports::new();
    for statement in &block.body {
        let Statement::ExportDeclaration(export) = statement else {
            continue;
        };
        match &export.declaration {
            Declaration::VariableDeclaration(variables) => {
                for variable in &variables.declarations {
                    let name = variable.id.get_identifier_name().unwrap().to_string();
                    let methods = match variable
                        .type_annotation
                        .as_ref()
                        .map(|annotation| &annotation.type_annotation)
                    {
                        Some(TSType::TSTypeLiteral(literal)) => literal
                            .members
                            .iter()
                            .filter_map(|member| match member {
                                TSSignature::TSMethodSignature(method) => match &method.key {
                                    PropertyKey::StaticIdentifier(key) => Some(key.name.to_string()),
                                    _ => None,
                                },
                                _ => None,
                            })
                            .collect(),
                        _ => BTreeSet::new(),
                    };
                    exports.insert(name, methods);
                }
            }
            Declaration::FunctionDeclaration(function) => {
                exports.insert(function.id.as_ref().unwrap().name.to_string(), BTreeSet::new());
            }
            Declaration::ClassDeclaration(class) => {
                exports.insert(class.id.as_ref().unwrap().name.to_string(), BTreeSet::new());
            }
            _ => {}
        }
    }
    exports
}

/// What the host's `dupin` module exports, with the methods of each frozen object it exports.
fn provided() -> Exports {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        let (module, _) = Module::declare(ctx, HOST_MODULE, host::SOURCE)
            .unwrap()
            .eval()
            .unwrap();
        let namespace = module.namespace().unwrap();
        namespace
            .keys::<String>()
            .map(|name| {
                let name = name.unwrap();
                let value: Value = namespace.get(name.as_str()).unwrap();
                let methods = match value.as_object() {
                    Some(object) if !value.is_function() => methods(object),
                    _ => BTreeSet::new(),
                };
                (name, methods)
            })
            .collect()
    })
}

fn methods(object: &Object<'_>) -> BTreeSet<String> {
    object
        .keys::<String>()
        .map(Result::unwrap)
        .filter(|key| object.get::<_, Value>(key.as_str()).unwrap().is_function())
        .collect()
}

#[test]
fn declarations_match_what_the_host_module_exports() {
    assert_eq!(
        declared(),
        provided(),
        "sdk/dupin.d.ts must declare what the host's dupin module exports"
    );
}

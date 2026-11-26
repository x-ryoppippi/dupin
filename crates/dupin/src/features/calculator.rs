use std::sync::Arc;

use dupin_core::{Batch, CommandError, CommandSpec, Config, Item, Provider, ProviderError, Request};

use super::extensions::Port;
use super::{Bundled, Feature, default_query};
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::ui::Ui;

pub(crate) const CALCULATOR: &str = "calculator.evaluate";
/// The bundled extension, in extensions/calculator, that answers calculations.
pub(crate) const PACKAGE: &str = "calculator";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    reload,
    bundled: Some(Bundled {
        package: PACKAGE,
        command: CALCULATOR,
        paste_notice: None,
        enabled: |config| config.calculator,
        turn_on: |config| config.calculator = true,
        main_search: true,
    }),
    ..Feature::EMPTY
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    if config.calculator {
        let command = id(CALCULATOR);
        commands.registry.register(CommandSpec::new(
            command.clone(),
            1,
            presentation("Calculate", None, None, None, false, 0.0),
        ))?;
        commands.defaults.push(command);
    }
    Ok(())
}

fn start(providers: &mut Providers) {
    let port = providers.port(CALCULATOR);
    providers.add(CALCULATOR, Arc::new(Calculations { port }));
}

fn reload(ui: &mut Ui, config: &Config, _: &mut ConfiguredCommands) -> bool {
    ui.currency
        .set_enabled(config.calculator && config.calculator_currency)
        && default_query(ui)
}

/// The calculator in the main search: the bundled extension's answer to what is typed there.
/// Every calculation has a digit in it, so the package never sees, nor is run for, a query
/// without one. Unlike under a keyword, a package that is off or failing shows nothing here.
struct Calculations {
    port: Arc<Port>,
}

impl Provider for Calculations {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        self.search_batches(request, &mut |batch| emit(batch.items))
    }

    fn search_batches(&self, request: &Request, emit: &mut dyn FnMut(Batch)) -> Result<(), ProviderError> {
        if request.limit == 0 || request.is_cancelled() || !could_calculate(&request.query) {
            return Ok(());
        }
        match self.port.provider() {
            Some(extension) => extension.search_batches(request, emit),
            None => Ok(()),
        }
    }
}

fn could_calculate(query: &str) -> bool {
    query.bytes().any(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use dupin_core::{Action, View};
    use dupin_extension_bridge::PackageId;

    use super::*;

    #[derive(Default)]
    struct Counted(AtomicUsize);

    impl Provider for Counted {
        fn search(&self, _: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            emit(vec![Item::path("/36".into(), "36".into(), Vec::new())]);
            Ok(())
        }
    }

    fn answers(calculations: &Calculations, query: &str) -> usize {
        let mut rows = 0;
        calculations
            .search(&Request::detached(query, 1), &mut |batch| rows += batch.len())
            .unwrap();
        rows
    }

    #[test]
    fn only_a_query_with_a_digit_reaches_the_package() {
        let calculations = Calculations { port: Arc::default() };
        assert_eq!(
            answers(&calculations, "12*3"),
            0,
            "nothing while it hasn't loaded"
        );
        let package = Arc::new(Counted::default());
        calculations.port.set(Some(package.clone()));
        for query in ["safari", "pi", "km to mi", "€"] {
            assert_eq!(answers(&calculations, query), 0, "{query}");
        }
        assert_eq!(package.0.load(Ordering::Relaxed), 0);
        assert_eq!(answers(&calculations, "12*3"), 1);
        assert_eq!(package.0.load(Ordering::Relaxed), 1);
    }

    /// The bundled package, answering the main search with Dupin's own calculator, as the
    /// launcher runs it; `bundled` says whether it is the shipped package or a copy with nothing
    /// granted.
    fn main_search(bundled: bool) -> (Calculations, dupin_extension_bridge::Bridge, PackageId) {
        let folder = crate::extensions::bundled::directory().unwrap().join(PACKAGE);
        let extension = if bundled {
            crate::extensions::load_bundled(&folder)
        } else {
            crate::extensions::load_folder(&folder)
        }
        .unwrap();
        let bridge = dupin_extension_bridge::Bridge::new(
            dupin_extension_bridge::Broker::new(Default::default(), Default::default()),
            None,
            dupin_extension_bridge::Services {
                calculator: Box::new(crate::calculator::Calculator::default()),
                ..dupin_extension_bridge::Services::none()
            },
        );
        if bundled {
            bridge.set_bundled(&extension.package, extension.revision.clone(), true);
            bridge.install_bundled(
                &extension.package,
                extension.revision.clone(),
                extension.capabilities.clone(),
            );
        } else {
            assert_eq!(extension.package, PackageId::new(PACKAGE).unwrap());
            bridge.install(
                &extension.package,
                extension.revision.clone(),
                extension.capabilities.clone(),
            );
            bridge.set_enabled(&extension.package, true).unwrap();
        }
        let package = extension.package.clone();
        let calculations = Calculations { port: Arc::default() };
        calculations
            .port
            .set(Some(Arc::new(crate::extensions::ExtensionProvider::with_bridge(
                extension,
                crate::extensions::Role::MainSearch,
                bridge.clone(),
            ))));
        (calculations, bridge, package)
    }

    fn rows(calculations: &Calculations, query: &str) -> Vec<Item> {
        let mut rows = Vec::new();
        calculations
            .search(&Request::detached(query, 8), &mut |batch| rows.extend(batch))
            .unwrap();
        rows
    }

    #[test]
    fn the_bundled_package_answers_calculations_as_the_native_calculator_did() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (calculations, _, _) = main_search(true);
        for (query, result, number) in [
            ("12*3", "36", "36"),
            (" 5 km to mi ", "approx. 3.1068559612 mi", "3.1068559612"),
            ("100 f to c", "approx. 37.7777777778 °C", "37.7777777778"),
            ("(20+12) °F in c", "0 °C", "0"),
            ("2 m^2 in cm^2", "20000 cm^2", "20000"),
        ] {
            let found = rows(&calculations, query);
            let [row] = found.as_slice() else {
                panic!("{query}: {} rows", found.len())
            };
            assert_eq!(
                row.view,
                View::Pair {
                    source: query.trim().into(),
                    result: result.into()
                },
                "{query}"
            );
            assert_eq!(row.id, "answer");
            assert!(!row.record_usage);
            assert_eq!(row.primary.as_deref(), Some("copy"));
            assert_eq!(row.actions[0].title, "Copy number");
            assert_eq!(row.action("copy"), Some(&Action::Copy(number.into())), "{query}");
        }
        for query in [
            "safari",
            "42",
            "12*",
            "1Password",
            "100 usd to jpy",
            "2^2^40",
            "1<<1000000000000",
        ] {
            assert!(rows(&calculations, query).is_empty(), "{query}");
        }
    }

    #[test]
    fn a_copy_without_the_query_grant_never_sees_the_main_search() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (calculations, bridge, package) = main_search(false);
        assert!(rows(&calculations, "12*3").is_empty());
        let asked: Vec<_> = bridge
            .pending(Some(&package))
            .iter()
            .map(|request| request.scope.key().to_string())
            .collect();
        assert_eq!(
            asked,
            ["query"],
            "it waits in Settings, never asking in the panel"
        );
        for request in bridge.pending(Some(&package)) {
            bridge
                .allow(&package, &request.revision, &request.scope, None)
                .unwrap();
        }
        assert!(rows(&calculations, "12*3").is_empty(), "nor works anything out");
        for request in bridge.pending(Some(&package)) {
            bridge
                .allow(&package, &request.revision, &request.scope, None)
                .unwrap();
        }
        assert_eq!(rows(&calculations, "12*3").len(), 1);
        bridge
            .revoke(&package, &dupin_extension_bridge::ScopeKey::Query)
            .unwrap();
        assert!(rows(&calculations, "12*3").is_empty(), "revoked at once");
    }
}

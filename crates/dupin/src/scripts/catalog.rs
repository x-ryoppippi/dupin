use std::collections::BTreeMap;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use dupin_core::script::{Header, ScriptRun};

const MAX_SCRIPTS: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Script {
    pub path: PathBuf,
    pub header: Header,
}

impl Script {
    pub fn run(&self, args: Vec<String>) -> ScriptRun {
        ScriptRun {
            path: self.path.clone(),
            title: self.header.title.clone(),
            args,
            mode: self.header.mode,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub scripts: BTreeMap<PathBuf, Script>,
    pub notice: Option<String>,
}

impl Catalog {
    pub fn bound_script(&self, name: &str, args: &[String]) -> Result<&Script, String> {
        let script = self
            .scripts
            .values()
            .find(|script| script.path.file_name().is_some_and(|file| file == name))
            .ok_or_else(|| format!("Script {name:?} was not found in the scripts directory"))?;
        script.header.bind_args(args)?;
        Ok(script)
    }

    pub fn scan(directory: &Path) -> Self {
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                return Self {
                    notice: Some(format!("Couldn't read scripts: {error}")),
                    ..Self::default()
                };
            }
        };
        let mut problems = Vec::new();
        let mut paths: Vec<_> = entries
            .filter_map(|entry| match entry {
                Ok(entry) => {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    (!name.starts_with('.') && !name.ends_with('~') && !name.ends_with(".swp"))
                        .then(|| entry.path())
                }
                Err(error) => {
                    problems.push(format!("Couldn't read a script entry: {error}"));
                    None
                }
            })
            .collect();
        paths.sort();
        let mut scripts = BTreeMap::new();
        for path in paths {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let metadata = match path.metadata() {
                Ok(metadata) if metadata.is_file() => metadata,
                Ok(_) => continue,
                Err(error) => {
                    problems.push(format!("{name}: {error}"));
                    continue;
                }
            };
            if metadata.permissions().mode() & 0o111 == 0 {
                problems.push(format!("{name}: not executable; use chmod +x"));
                continue;
            }
            if scripts.len() == MAX_SCRIPTS {
                problems.push("Only the first 200 valid scripts are available.".into());
                break;
            }
            let mut bytes = Vec::new();
            if let Err(error) =
                std::fs::File::open(&path).and_then(|file| file.take(8192).read_to_end(&mut bytes))
            {
                problems.push(format!("{name}: {error}"));
                continue;
            }
            let fallback = path.file_stem().unwrap_or_default().to_string_lossy();
            match Header::parse(&bytes, &fallback) {
                Ok((mut header, warnings)) => {
                    problems.extend(warnings.into_iter().map(|warning| format!("{name}: {warning}")));
                    if let Some(icon) = &header.icon {
                        header.icon = Some(
                            if let Some(relative) = icon.to_string_lossy().strip_prefix("~/") {
                                std::env::var_os("HOME")
                                    .map(PathBuf::from)
                                    .map(|home| home.join(relative))
                                    .unwrap_or_else(|| icon.clone())
                            } else if icon.is_relative() {
                                directory.join(icon)
                            } else {
                                icon.clone()
                            },
                        );
                    }
                    scripts.insert(path.clone(), Script { path, header });
                }
                Err(problem) => problems.push(format!("{name}: {problem}")),
            }
        }
        let notice = (!problems.is_empty()).then(|| {
            let mut text = problems.iter().take(20).cloned().collect::<Vec<_>>().join("\n");
            if problems.len() > 20 {
                text.push_str(&format!("\n{} more script problems", problems.len() - 20));
            }
            text
        });
        Self { scripts, notice }
    }
}

impl dupin_host_services::exec::Scripts for Catalog {
    fn resolve(&self, name: &str, args: &[String]) -> Result<PathBuf, String> {
        self.bound_script(name, args).map(|script| script.path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "dupin-scripts-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn write(&self, name: &str, source: &str, mode: u32) {
            let path = self.0.join(name);
            std::fs::write(&path, source).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovery_never_executes_and_filters_backups_and_non_executable_files() {
        let root = Fixture::new();
        let marker = root.0.join("executed");
        let body = format!(
            "#!/bin/sh\n# @dupin.title Echo\n# @dupin.icon icon.svg\n# @dupin.future option\ntouch '{}'\n",
            marker.display()
        );
        root.write("echo.sh", &body, 0o700);
        root.write("plain.sh", "#!/bin/sh\nexit 99\n", 0o700);
        root.write(".hidden", &body, 0o700);
        root.write("backup~", &body, 0o700);
        root.write("backup.swp", &body, 0o700);
        root.write("disabled", &body, 0o600);
        root.write("invalid", "# @dupin.output invalid", 0o700);
        std::os::unix::fs::symlink(root.0.join("echo.sh"), root.0.join("linked")).unwrap();
        std::fs::create_dir(root.0.join("folder")).unwrap();
        for _ in 0..3 {
            let catalog = Catalog::scan(&root.0);
            assert_eq!(catalog.scripts.len(), 3);
            assert!(catalog.notice.as_ref().unwrap().contains("chmod +x"));
            assert!(catalog.notice.as_ref().unwrap().contains("unknown header key"));
            assert_eq!(
                catalog.scripts[&root.0.join("echo.sh")].header.icon,
                Some(root.0.join("icon.svg"))
            );
            let provider =
                super::super::provider::ScriptsProvider(std::sync::Arc::new(std::sync::RwLock::new(catalog)));
            let mut rows = Vec::new();
            dupin_core::Provider::search(
                &provider,
                &dupin_core::Request::detached("echo", 1),
                &mut |items| rows.extend(items),
            )
            .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].icon, Some(root.0.join("icon.svg")));
            assert!(matches!(
                rows[0].action("run"),
                Some(dupin_core::Action::RunScript(_))
            ));
            rows.clear();
            dupin_core::Provider::search(
                &provider,
                &dupin_core::Request::detached("plain", 1),
                &mut |items| rows.extend(items),
            )
            .unwrap();
            assert_eq!(rows[0].icon, Some(root.0.join("plain.sh")));
            assert!(!marker.exists());
        }
    }

    #[test]
    fn script_bindings_use_catalogue_filenames_and_header_arguments() {
        use dupin_core::script::OutputMode;

        let root = Fixture::new();
        let header = "#!/bin/sh\n# @dupin.output full\n# @dupin.argument message\n# @dupin.argument note?\n";
        root.write("fixture.sh", header, 0o600);
        let args = vec!["a b".to_owned(), "$HOME * ~".to_owned()];
        assert!(Catalog::scan(&root.0).bound_script("fixture.sh", &args).is_err());
        root.write("fixture.sh", header, 0o700);

        let catalog = Catalog::scan(&root.0);
        let run = catalog
            .bound_script("fixture.sh", &args)
            .unwrap()
            .run(args.clone());
        assert_eq!(run.path, root.0.join("fixture.sh"));
        assert_eq!(run.args, args);
        assert_eq!(run.mode, OutputMode::Full);
        assert!(catalog.bound_script("../fixture.sh", &args).is_err());
        assert!(catalog.bound_script("fixture.sh", &["bad\0arg".into()]).is_err());
        assert!(
            catalog
                .bound_script("fixture.sh", &[])
                .unwrap_err()
                .contains("Needs message")
        );

        root.write("fixture.sh", "#!/bin/sh\n# @dupin.output silent\n", 0o700);
        let catalog = Catalog::scan(&root.0);
        assert!(catalog.bound_script("fixture.sh", &[]).is_ok());
        assert!(
            catalog
                .bound_script("fixture.sh", &args)
                .unwrap_err()
                .contains("Too many arguments")
        );
    }

    #[test]
    fn script_limit_applies_after_hidden_and_invalid_files_are_skipped() {
        let root = Fixture::new();
        root.write("000-invalid", "# @dupin.output invalid", 0o700);
        for index in 0..201 {
            root.write(&format!("script-{index:03}"), "#!/bin/sh\nexit 0", 0o700);
        }
        let catalog = Catalog::scan(&root.0);
        assert_eq!(catalog.scripts.len(), 200);
        assert!(catalog.notice.unwrap().contains("first 200"));
        assert!(!catalog.scripts.contains_key(&root.0.join("script-200")));
        assert!(Catalog::scan(&root.0.join("missing")).notice.is_none());
    }
}

pub mod catalog;
pub mod entry;
pub mod output;
pub mod provider;
pub mod runner;

use crate::catalog_watch::{Depth, Watch};

fn watch_catalog(
    directory: std::path::PathBuf,
    deliver: impl Fn(catalog::Catalog) + Send + 'static,
) -> std::io::Result<Watch> {
    Watch::start(
        directory,
        Depth::Entries,
        |directory, watch_problem| {
            let mut catalog = catalog::Catalog::scan(directory);
            if let Some(problem) = watch_problem {
                catalog.notice = Some(match catalog.notice {
                    Some(notice) => format!("{notice}\n{problem}"),
                    None => problem,
                });
            }
            Some(catalog)
        },
        deliver,
    )
}

pub fn watch(panel: rune::AppHandle<crate::ui::Ui>) -> Option<Watch> {
    let directory = crate::settings::path()?.parent()?.join("scripts");
    match watch_catalog(directory, move |catalog| {
        panel.update(move |s, tx| s.scripts_catalog(catalog, tx));
    }) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            crate::log::warn(&format!("Couldn't start script watcher: {error}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn moved_script_directory_is_rewatched_before_later_edits() {
        let root = std::env::temp_dir().join(format!(
            "dupin-script-watch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory = root.join("scripts");
        std::fs::create_dir_all(&directory).unwrap();
        let write = |title: &str| {
            let file = directory.join("fixture");
            std::fs::write(&file, format!("#!/bin/sh\n# @dupin.title {title}\nexit 99\n")).unwrap();
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o700)).unwrap();
        };
        write("Initial");
        let (send, received) = mpsc::channel();
        let watcher = watch_catalog(directory.clone(), move |catalog| {
            let _ = send.send(catalog);
        })
        .unwrap();
        let wait_for = |title: &str| {
            let deadline = Instant::now() + Duration::from_secs(6);
            loop {
                let catalog = received
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .unwrap();
                if catalog
                    .scripts
                    .values()
                    .any(|script| script.header.title == title)
                {
                    break;
                }
            }
        };
        wait_for("Initial");
        std::fs::rename(&directory, root.join("old-scripts")).unwrap();
        std::fs::create_dir(&directory).unwrap();
        write("Replacement");
        wait_for("Replacement");
        write("Edited after replacement");
        wait_for("Edited after replacement");
        drop(watcher);
        std::fs::remove_dir_all(root).unwrap();
    }
}

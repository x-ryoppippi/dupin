pub mod catalog;
pub mod entry;
pub mod provider;
pub mod runner;

pub fn watch(panel: rune::AppHandle<crate::ui::Ui>) -> Option<dupin_mac::watcher::PathWatcher> {
    let directory = crate::settings::path()?.parent()?.join("scripts");
    let (send, received) = std::sync::mpsc::channel();
    let changed = send.clone();
    let watcher = dupin_mac::watcher::PathWatcher::start_shallow(vec![directory.clone()], move |_| {
        let _ = changed.send(());
    });
    let _ = send.send(());
    std::thread::Builder::new()
        .name("dupin-script-catalog".into())
        .spawn(move || {
            while received.recv().is_ok() {
                while received.try_recv().is_ok() {}
                let catalog = catalog::Catalog::scan(&directory);
                panel.update(move |s, tx| s.scripts_catalog(catalog, tx));
            }
        })
        .expect("spawn script catalogue");
    match watcher {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            crate::log::warn(error);
            None
        }
    }
}
pub mod output;

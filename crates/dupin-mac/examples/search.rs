//! Run a provider from the command line, through a `Searcher` as the app does:
//! `cargo run -p dupin-mac --example search -- apps saf`, `-- files invoice` or `-- in invoice`.

#[cfg(target_os = "macos")]
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::mpsc;
#[cfg(target_os = "macos")]
use std::time::Instant;

#[cfg(target_os = "macos")]
use dupin_core::{Delivery, Provider, Searcher};
#[cfg(target_os = "macos")]
use dupin_mac::{AppsProvider, SpotlightProvider, Target};

#[cfg(target_os = "macos")]
fn main() {
    let mut args = std::env::args().skip(1);
    let kind = args.next().unwrap_or_default();
    let query = args.collect::<Vec<_>>().join(" ");
    let provider: Arc<dyn Provider> = match kind.as_str() {
        "apps" => Arc::new(AppsProvider::standard()),
        "files" => Arc::new(SpotlightProvider::new(Target::Names)),
        "in" => Arc::new(SpotlightProvider::new(Target::Contents)),
        _ => return eprintln!("usage: search apps|files|in <query>"),
    };
    let (tx, rx) = mpsc::channel();
    let searcher = Searcher::spawn(&kind, provider, move |d| drop(tx.send(d)));
    let start = Instant::now();
    searcher.search(1, query, None, None, 50);
    for delivery in rx {
        match delivery {
            Delivery::Batch { items, .. }
            | Delivery::Presented {
                batch: dupin_core::Batch { items, .. },
                ..
            } => {
                println!("-- batch of {} at {:?}", items.len(), start.elapsed());
                for item in items {
                    println!("{}\t{:?}", item.display_title(), item.view);
                }
            }
            Delivery::Done { .. } | Delivery::Failed { .. } | Delivery::Expired { .. } => {
                println!("-- done at {:?}", start.elapsed());
                break;
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("this example runs on macOS only");
}

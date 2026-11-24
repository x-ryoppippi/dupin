//! `DUPIN_LATENCY=<file>` in a headless run: for each edit of the query, the time from the edit
//! reaching the session to the first frame encoded after every provider for it has delivered,
//! split into stages, as one JSON line. `scripts/latency.nu` drives the runs and reads the lines.
//!
//! Stages, which add up to `total_ms`:
//! - `dispatch_ms`: from the edit to the searches it starts being handed to their workers.
//! - `provider_ms`: from there to the last provider's final delivery leaving its worker.
//! - `merge_ms`: from there to the session holding the finished, ranked results.
//! - `layout_ms`: from there to the next frame being built, laid out and encoded.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use dupin_core::CommandId;

struct Probe {
    out: BufWriter<File>,
    sample: Option<Sample>,
}

struct Sample {
    /// The headless clock when the edit arrived, so the driver can tell which scripted key it was.
    t: f64,
    query: String,
    command: String,
    edited: Instant,
    dispatched: Option<Instant>,
    pending: BTreeMap<CommandId, u64>,
    /// The extension package behind each searched provider an extension answers, since a
    /// built-in's command ID stays the same whether its port answers it or the feature says
    /// the port hasn't loaded.
    packages: BTreeMap<String, String>,
    providers: BTreeMap<String, f64>,
    last_provider: Option<Instant>,
    done: Option<Instant>,
    /// Rows per provider in the finished results; with `pending`'s providers, what lets the
    /// driver tell which feature answered.
    shown: BTreeMap<String, usize>,
}

thread_local! {
    static PROBE: RefCell<Option<Probe>> = RefCell::new(open());
}

fn open() -> Option<Probe> {
    // Lines carry query text, which a real panel must never write anywhere.
    if !rune::tooling::is_headless() {
        return None;
    }
    let path = std::env::var_os("DUPIN_LATENCY")?;
    let file = File::create(&path)
        .unwrap_or_else(|error| panic!("DUPIN_LATENCY {}: {error}", path.to_string_lossy()));
    Some(Probe {
        out: BufWriter::new(file),
        sample: None,
    })
}

fn with(f: impl FnOnce(&mut Probe)) {
    PROBE.with_borrow_mut(|probe| {
        if let Some(probe) = probe {
            f(probe);
        }
    });
}

fn ms(from: Instant, to: Instant) -> f64 {
    to.saturating_duration_since(from).as_secs_f64() * 1000.0
}

/// The query field's text changed and is about to reach the session.
pub fn edit_started(t: f64, query: &str) {
    with(|probe| {
        if let Some(stale) = probe.sample.take() {
            probe.write(&stale, None);
        }
        probe.sample = Some(Sample {
            t,
            query: query.to_owned(),
            command: String::new(),
            edited: Instant::now(),
            dispatched: None,
            pending: BTreeMap::new(),
            packages: BTreeMap::new(),
            providers: BTreeMap::new(),
            last_provider: None,
            done: None,
            shown: BTreeMap::new(),
        });
    });
}

/// The session took the edit and its searches have started. `settled` is true when nothing
/// is left to wait for, and `origins` are then the providers of the rows it shows.
pub fn edit_applied(command: &CommandId, settled: bool, origins: &[CommandId]) {
    with(|probe| {
        let Some(sample) = &mut probe.sample else { return };
        let now = Instant::now();
        sample.command = command.as_str().to_owned();
        sample.dispatched = Some(now);
        if settled || sample.pending.is_empty() {
            sample.finish(now, origins);
        }
    });
}

/// `package` is the extension package that answers `provider`, if one does.
pub fn search_started(provider: &CommandId, generation: u64, package: Option<&str>) {
    with(|probe| {
        if let Some(sample) = &mut probe.sample
            && sample.dispatched.is_none()
        {
            sample.pending.insert(provider.clone(), generation);
            if let Some(package) = package {
                sample
                    .packages
                    .insert(provider.as_str().to_owned(), package.to_owned());
            }
        }
    });
}

/// A delivery from `provider`'s worker, sent at `sent`, reached the session. `pending` says
/// whether the provider still owes results for a generation.
pub fn delivered(provider: &CommandId, sent: Instant, pending: impl Fn(u64) -> bool) {
    with(|probe| {
        let Some(sample) = &mut probe.sample else { return };
        let Some(&generation) = sample.pending.get(provider) else {
            return;
        };
        if pending(generation) || sample.providers.contains_key(provider.as_str()) {
            return;
        }
        let Some(dispatched) = sample.dispatched else {
            return;
        };
        sample
            .providers
            .insert(provider.as_str().to_owned(), ms(dispatched, sent));
        sample.last_provider = Some(sample.last_provider.map_or(sent, |last| last.max(sent)));
    });
}

/// Every provider for the current query has finished and the session has merged them into
/// rows from `origins`.
pub fn results_done(origins: &[CommandId]) {
    with(|probe| {
        if let Some(sample) = &mut probe.sample
            && sample.dispatched.is_some()
            && sample.done.is_none()
        {
            sample.finish(Instant::now(), origins);
        }
    });
}

impl Sample {
    fn finish(&mut self, at: Instant, origins: &[CommandId]) {
        self.done = Some(at);
        self.shown.clear();
        for origin in origins {
            *self.shown.entry(origin.as_str().to_owned()).or_default() += 1;
        }
    }
}

/// `App::on_frame_encoded`.
pub fn frame(f: &rune::FrameEncoded) {
    with(|probe| {
        if probe
            .sample
            .as_ref()
            .and_then(|sample| sample.done)
            .is_some_and(|done| f.at() >= done)
        {
            let sample = probe.sample.take().expect("checked above");
            probe.write(&sample, Some(f.at()));
        }
    });
}

impl Probe {
    /// `drawn` is None for an edit that the next one replaced before its results were drawn.
    fn write(&mut self, sample: &Sample, drawn: Option<Instant>) {
        let line = match (drawn, sample.dispatched, sample.done) {
            (Some(drawn), Some(dispatched), Some(done)) => {
                let provided = sample.last_provider.unwrap_or(dispatched).clamp(dispatched, done);
                serde_json::json!({
                    "t": sample.t,
                    "query": sample.query,
                    "command": sample.command,
                    "total_ms": ms(sample.edited, drawn),
                    "dispatch_ms": ms(sample.edited, dispatched),
                    "provider_ms": ms(dispatched, provided),
                    "merge_ms": ms(provided, done),
                    "layout_ms": ms(done, drawn),
                    "providers": sample.providers,
                    "searched": sample.pending.keys().map(CommandId::as_str).collect::<Vec<_>>(),
                    "packages": sample.packages,
                    "shown": sample.shown,
                })
            }
            _ => serde_json::json!({
                "t": sample.t,
                "query": sample.query,
                "command": sample.command,
                "superseded": true,
            }),
        };
        let written = writeln!(self.out, "{line}").and_then(|()| self.out.flush());
        if let Err(error) = written {
            crate::log::warn(&format!("couldn't write a latency sample: {error}"));
        }
    }
}

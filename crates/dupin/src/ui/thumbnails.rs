//! The images extension rows name by a handle: read from the package's own storage through the
//! bridge, decoded and scaled down with ImageIO on a worker thread, then handed to Rune's image
//! cache. Only the rows in or near view, and the selected row's preview, are asked for, so a
//! history of thousands of images holds a screenful of thumbnails.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use dupin_core::{HostImage, HostImageSource};
use dupin_extension_bridge::{Bridge, ImageData, ImageRef, PackageId};
use rune::{AppHandle, Image};

use super::Ui;

/// A row's icon, at twice [`super::ICON_SIZE`] for a Retina display.
const ROW_PIXELS: (u32, u32) = (64, 64);
/// A detail's preview: [`PREVIEW_HEIGHT`] points high and as wide as the panel's card, at twice
/// that for a Retina display.
const PREVIEW_PIXELS: (u32, u32) = (1280, 2 * PREVIEW_HEIGHT as u32);
pub const PREVIEW_HEIGHT: f32 = 200.0;
/// Previews kept decoded: the selected row's and the last few, for moving back and forth.
const MAX_PREVIEWS: usize = 3;
/// Images that failed are not asked for again until this many have, or access changed.
const MAX_FAILED: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Size {
    Row,
    Preview,
}

impl Size {
    fn pixels(self) -> (u32, u32) {
        match self {
            Size::Row => ROW_PIXELS,
            Size::Preview => PREVIEW_PIXELS,
        }
    }
}

type Key = (HostImage, Size);

/// What a detail shows for its image.
pub enum Preview<'a> {
    Ready(&'a Image),
    /// Not made yet: the space is kept, so the card doesn't jump when it lands.
    Loading,
    /// It can't be made, as when the package may not read its storage or the image doesn't
    /// decode, so no space is kept for it.
    Failed,
}

pub(super) struct Job {
    pub image: HostImage,
    pub size: Size,
    pub package: PackageId,
}

#[derive(Default)]
struct Queue {
    /// Newest wishes first; replaced whenever the rows in view change.
    waiting: VecDeque<Job>,
    /// The job the worker is on.
    running: Option<Key>,
    /// Whether a worker thread is draining the queue.
    worker: bool,
}

pub(crate) struct Thumbnails {
    queue: Arc<Mutex<Queue>>,
    previews: HashMap<HostImage, Image>,
    /// The previews in `previews`, least recently asked for first.
    order: VecDeque<HostImage>,
    failed: HashSet<Key>,
}

impl Thumbnails {
    pub(super) fn new() -> Self {
        Self {
            queue: Arc::default(),
            previews: HashMap::new(),
            order: VecDeque::new(),
            failed: HashSet::new(),
        }
    }

    pub(crate) fn preview(&self, image: &HostImage) -> Preview<'_> {
        match self.previews.get(image) {
            Some(preview) => Preview::Ready(preview),
            None if self.failed.contains(&(image.clone(), Size::Preview)) => Preview::Failed,
            None => Preview::Loading,
        }
    }

    pub(super) fn has_preview(&self, image: &HostImage) -> bool {
        self.previews.contains_key(image)
    }

    /// Forgets which images failed, so they are asked for again, as after a package's access
    /// changed.
    pub(super) fn retry_failed(&mut self) {
        self.failed.clear();
    }

    /// Asks the worker for `jobs`, in order, in place of whatever it hadn't started: an image
    /// scrolled out of view before its turn is never read.
    pub(super) fn want(&mut self, jobs: Vec<Job>, bridge: &Bridge, me: &AppHandle<Ui>) {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        queue.waiting = jobs
            .into_iter()
            .filter(|job| {
                let key = (job.image.clone(), job.size);
                !self.failed.contains(&key) && queue.running.as_ref() != Some(&key)
            })
            .collect();
        if queue.waiting.is_empty() || queue.worker {
            return;
        }
        queue.worker = true;
        drop(queue);
        let (queue, bridge, me) = (self.queue.clone(), bridge.clone(), me.clone());
        let spawned = std::thread::Builder::new()
            .name("thumbnails".into())
            .spawn(move || work(&queue, &bridge, &me));
        if spawned.is_err() {
            self.queue.lock().unwrap_or_else(PoisonError::into_inner).worker = false;
        }
    }

    /// Takes a finished preview, keeping only the last few.
    pub(super) fn keep_preview(&mut self, image: HostImage, decoded: Image) {
        self.order.retain(|kept| *kept != image);
        self.order.push_back(image.clone());
        self.previews.insert(image, decoded);
        while self.order.len() > MAX_PREVIEWS {
            if let Some(oldest) = self.order.pop_front() {
                self.previews.remove(&oldest);
            }
        }
    }

    pub(super) fn failed(&mut self, image: HostImage, size: Size) {
        if self.failed.len() >= MAX_FAILED {
            self.failed.clear();
        }
        self.failed.insert((image, size));
    }
}

/// Makes each waiting thumbnail in turn, and stops once none is left.
fn work(queue: &Mutex<Queue>, bridge: &Bridge, me: &AppHandle<Ui>) {
    loop {
        let job = {
            let mut queue = queue.lock().unwrap_or_else(PoisonError::into_inner);
            queue.running = None;
            let Some(job) = queue.waiting.pop_front() else {
                queue.worker = false;
                return;
            };
            queue.running = Some((job.image.clone(), job.size));
            job
        };
        let png = thumbnail(bridge, &job);
        let Job { image, size, .. } = job;
        me.update(move |ui, _| ui.thumbnail_made(image, size, png));
    }
}

/// The PNG of `job`'s image at its size, or `None` when the package may not read its storage,
/// the handle isn't the package's, or the image can't be decoded.
fn thumbnail(bridge: &Bridge, job: &Job) -> Option<Vec<u8>> {
    let image = match &job.image.source {
        HostImageSource::Blob(handle) => ImageRef::Blob(handle),
        HostImageSource::File { path, handle } => ImageRef::File {
            path: path.to_str()?,
            handle,
        },
    };
    let fit = job.size.pixels();
    let made = bridge.read_image(&job.package, &image, |data| match data {
        ImageData::Bytes(bytes) => dupin_mac::thumbnail::from_bytes(bytes, fit),
        ImageData::File(path) => dupin_mac::thumbnail::from_file(path, fit),
    });
    match made {
        Ok(png) => png,
        Err(failure) => {
            if !matches!(failure.code(), "needs-consent" | "disabled" | "not-found") {
                crate::log::warn(&format!(
                    "extension {}: couldn't draw an image: {failure}",
                    job.image.extension
                ));
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_that_failed_keeps_no_space_until_access_changes() {
        let image = HostImage {
            extension: "clipboard-history".into(),
            source: HostImageSource::Blob("handle".into()),
        };
        let mut thumbnails = Thumbnails::new();
        assert!(matches!(thumbnails.preview(&image), Preview::Loading));
        thumbnails.failed(image.clone(), Size::Row);
        assert!(matches!(thumbnails.preview(&image), Preview::Loading));
        thumbnails.failed(image.clone(), Size::Preview);
        assert!(matches!(thumbnails.preview(&image), Preview::Failed));
        thumbnails.retry_failed();
        assert!(matches!(thumbnails.preview(&image), Preview::Loading));
    }
}

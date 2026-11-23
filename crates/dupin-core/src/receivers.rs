use std::path::PathBuf;

use crate::{ItemAction, Payload};

/// Builds actions for the inputs a result can pass to receivers.
pub struct ReceiverActions<'a> {
    payload: &'a Payload,
    actions: Vec<ItemAction>,
}

impl<'a> ReceiverActions<'a> {
    pub fn new(payload: &'a Payload) -> Self {
        Self {
            payload,
            actions: Vec::new(),
        }
    }

    pub fn for_text<I: IntoIterator<Item = ItemAction>>(&mut self, build: impl FnOnce(&'a str) -> I) {
        if let Some(text) = self.payload.nonblank_text() {
            self.actions.extend(build(text));
        }
    }

    pub fn for_paths<I: IntoIterator<Item = ItemAction>>(&mut self, build: impl FnOnce(&'a [PathBuf]) -> I) {
        if !self.payload.paths.is_empty() {
            self.actions.extend(build(&self.payload.paths));
        }
    }

    pub fn finish(self) -> Vec<ItemAction> {
        self.actions
    }
}

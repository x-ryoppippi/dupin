//! Sleep, restart and the other system commands, which Dupin doesn't run on Linux yet.

use std::sync::atomic::{AtomicBool, Ordering};

use dupin_core::{Item, Provider, ProviderError, Request, SystemCommand};

pub type Runner = fn(SystemCommand) -> Result<(), String>;

pub const COMMANDS: [SystemCommand; 0] = [];

pub fn run(_command: SystemCommand) -> Result<(), String> {
    Err("System commands aren't supported on Linux yet.".into())
}

pub fn execute(
    command: SystemCommand,
    _automation_enabled: bool,
    runner: impl FnOnce(SystemCommand) -> Result<(), String>,
) -> Result<(), String> {
    runner(command)
}

pub struct SystemProvider {
    automation_enabled: AtomicBool,
}

impl SystemProvider {
    pub fn new(automation_enabled: bool) -> Self {
        Self {
            automation_enabled: AtomicBool::new(automation_enabled),
        }
    }

    pub fn set_automation_enabled(&self, enabled: bool) -> bool {
        self.automation_enabled.swap(enabled, Ordering::Relaxed) != enabled
    }
}

impl Provider for SystemProvider {
    fn search(&self, _: &Request, _: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        Ok(())
    }
}

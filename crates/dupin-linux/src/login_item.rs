//! Opening Dupin at login, which Dupin doesn't register on Linux yet.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
    Unknown,
}

/// Never made on Linux, so Settings leaves out the login row.
pub struct LoginItem(std::convert::Infallible);

impl LoginItem {
    pub fn status(&self) -> Status {
        match self.0 {}
    }

    pub fn set_enabled(&self, _enabled: bool) -> Result<(), String> {
        match self.0 {}
    }

    pub fn open_settings(&self) {
        match self.0 {}
    }
}

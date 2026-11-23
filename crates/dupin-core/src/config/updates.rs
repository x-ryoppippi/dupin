pub(super) const DEFAULT_ENABLED: bool = true;

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn updates_by_default() {
        assert!(Config::default().auto_update);
        assert!(!Config::parse("auto_update = false").unwrap().auto_update);
    }
}

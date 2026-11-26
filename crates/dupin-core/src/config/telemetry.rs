pub(super) const DEFAULT_ENABLED: bool = true;

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn sends_by_default_and_turns_off_from_the_file() {
        assert!(Config::default().telemetry);
        assert!(!Config::parse("telemetry = false").unwrap().telemetry);
    }
}

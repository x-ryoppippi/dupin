pub(super) const DEFAULT_LIMIT: usize = 200;

pub(super) fn default_demotions() -> Vec<String> {
    [
        "~/Library",
        "Caches",
        "node_modules",
        ".git",
        "target",
        "build",
        "dist",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn file_demotions_default_and_round_trip() {
        let defaults = Config::default();
        assert_eq!(
            defaults.file_demotions,
            [
                "~/Library",
                "Caches",
                "node_modules",
                ".git",
                "target",
                "build",
                "dist",
            ]
        );

        let config = Config::parse("file_demotions = [\"~/Library\", \"Vendor/Cache\", \"\"]").unwrap();
        assert_eq!(config.file_demotions, ["~/Library", "Vendor/Cache", ""]);
        let encoded = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&encoded).unwrap(), config);
    }
}

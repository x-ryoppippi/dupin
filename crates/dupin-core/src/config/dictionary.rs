pub(super) fn default_keywords() -> Vec<String> {
    vec!["define".into(), "d".into()]
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn dictionary_keywords_default_and_can_be_configured() {
        assert_eq!(Config::parse("").unwrap().dictionary_keywords, ["define", "d"]);
        let config = Config::parse("dictionary_keywords = [\"word\", \"lookup\"]").unwrap();
        assert_eq!(config.dictionary_keywords, ["word", "lookup"]);
    }
}

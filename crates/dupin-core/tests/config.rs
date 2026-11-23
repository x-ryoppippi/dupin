use dupin_core::Config;
use serde_json::Value;

#[test]
fn default_config_round_trips_with_existing_toml_and_json_layout() {
    let config = Config::default();

    let toml = include_str!("fixtures/default-config.toml.snap");
    assert_eq!(toml::to_string(&config).unwrap(), toml);
    assert_eq!(Config::parse(toml).unwrap(), config);

    let json = include_str!("fixtures/default-config.json");
    assert_eq!(
        serde_json::to_value(&config).unwrap(),
        serde_json::from_str::<Value>(json).unwrap()
    );
    assert_eq!(serde_json::from_str::<Config>(json).unwrap(), config);
}

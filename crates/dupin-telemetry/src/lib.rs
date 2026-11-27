//! The anonymous daily usage ping, shared by the launcher that sends it and the collector in
//! `telemetry/` that validates it.
//!
//! [`Ping`] is the single definition of the wire format. The collector never restates it:
//! `generate-telemetry-schema` renders it as the JSON Schema module the collector validates
//! against and derives its TypeScript type from, and a test fails while the committed module
//! differs from what this crate generates.

use serde::Serialize;

/// The one document Dupin sends, at most once per UTC day.
///
/// The collector strips unknown keys rather than rejecting them, so a field can be added here
/// before the collector is redeployed; removing or tightening one breaks the releases still
/// sending it.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Ping {
    pub event: PingEvent,
    /// Random 128-bit value as lowercase hex, generated once per installation.
    #[cfg_attr(feature = "schema", schemars(regex(pattern = "^[0-9a-f]{32}$")))]
    pub anonymous_id: String,
    /// The release's version, as its `CFBundleShortVersionString` gives it.
    #[cfg_attr(feature = "schema", schemars(regex(pattern = r"^[\w.+-]{1,64}$")))]
    pub version: String,
    pub channel: Channel,
    /// `macos` and the major version, such as `macos 26`.
    #[cfg_attr(feature = "schema", schemars(regex(pattern = r"^macos \d{1,3}$")))]
    pub os: String,
    /// The CPU architecture, as Rust names it.
    #[cfg_attr(feature = "schema", schemars(regex(pattern = r"^\w{1,32}$")))]
    pub arch: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PingEvent {
    DupinDailyActive,
}

/// The release channel the build was packaged for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Stable,
    Nightly,
}

/// Renders [`Ping`]'s JSON Schema as the TypeScript module the collector imports.
///
/// Subschemas are inlined so the module is one self-contained object, which is what
/// `json-schema-to-ts` needs to derive a type from it.
#[cfg(feature = "schema")]
pub fn generate_schema_module() -> String {
    let generator = schemars::r#gen::SchemaSettings::draft07()
        .with(|settings| {
            settings.inline_subschemas = true;
            settings.option_add_null_type = false;
        })
        .into_generator();
    let schema = generator.into_root_schema_for::<Ping>();
    let json = serde_json::to_string_pretty(&schema).expect("a schema serializes");
    format!(
        "// Generated from `crates/dupin-telemetry` by `just telemetry::generate-schema`.\n\
         // Do not edit; change the Rust `Ping` type instead.\n\
         export const pingJsonSchema = {json} as const;\n"
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn serializes_to_the_documented_wire_format() {
        let ping = Ping {
            event: PingEvent::DupinDailyActive,
            anonymous_id: "0".repeat(32),
            version: "0.5.0-nightly.1234+abc1234".into(),
            channel: Channel::Nightly,
            os: "macos 26".into(),
            arch: "aarch64".into(),
        };

        assert_eq!(
            serde_json::to_value(ping).unwrap(),
            json!({
                "event": "dupin_daily_active",
                "anonymousId": "00000000000000000000000000000000",
                "version": "0.5.0-nightly.1234+abc1234",
                "channel": "nightly",
                "os": "macos 26",
                "arch": "aarch64",
            })
        );
    }

    #[cfg(feature = "schema")]
    mod schema {
        use serde_json::{Value, json};

        use super::super::generate_schema_module;

        const COMMITTED: &str = include_str!("../../../telemetry/src/v1/ping-schema.gen.ts");

        fn schema_value() -> Value {
            let module = generate_schema_module();
            let start = module.find('{').unwrap();
            let end = module.rfind('}').unwrap();
            serde_json::from_str(&module[start..=end]).unwrap()
        }

        #[test]
        fn the_collector_validates_against_this_crate_s_schema() {
            assert!(
                COMMITTED == generate_schema_module(),
                "telemetry/src/v1/ping-schema.gen.ts is out of date; run `just telemetry::generate-schema`"
            );
        }

        #[test]
        fn the_schema_is_self_contained_and_requires_every_field() {
            let schema = schema_value();

            assert!(schema.get("definitions").is_none());
            assert_eq!(
                schema["required"],
                json!(["anonymousId", "arch", "channel", "event", "os", "version"])
            );
            assert_eq!(
                schema["properties"]["event"]["enum"],
                json!(["dupin_daily_active"])
            );
            assert_eq!(
                schema["properties"]["channel"]["enum"],
                json!(["stable", "nightly"])
            );
        }
    }
}

// Probe for the check job without the release build; not for merging.

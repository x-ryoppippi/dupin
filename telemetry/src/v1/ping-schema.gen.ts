// Generated from `crates/dupin-telemetry` by `just telemetry::generate-schema`.
// Do not edit; change the Rust `Ping` type instead.
export const pingJsonSchema = {
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "Ping",
  "description": "The one document Dupin sends, at most once per UTC day.\n\nThe collector strips unknown keys rather than rejecting them, so a field can be added here before the collector is redeployed; removing or tightening one breaks the releases still sending it.",
  "type": "object",
  "required": [
    "anonymousId",
    "arch",
    "channel",
    "event",
    "os",
    "version"
  ],
  "properties": {
    "anonymousId": {
      "description": "Random 128-bit value as lowercase hex, generated once per installation.",
      "type": "string",
      "pattern": "^[0-9a-f]{32}$"
    },
    "arch": {
      "description": "The CPU architecture, as Rust names it.",
      "type": "string",
      "pattern": "^\\w{1,32}$"
    },
    "channel": {
      "description": "The release channel the build was packaged for.",
      "type": "string",
      "enum": [
        "stable",
        "nightly"
      ]
    },
    "event": {
      "type": "string",
      "enum": [
        "dupin_daily_active"
      ]
    },
    "os": {
      "description": "`macos` and the major version, such as `macos 26`.",
      "type": "string",
      "pattern": "^macos \\d{1,3}$"
    },
    "version": {
      "description": "The release's version, as its `CFBundleShortVersionString` gives it.",
      "type": "string",
      "pattern": "^[\\w.+-]{1,64}$"
    }
  }
} as const;

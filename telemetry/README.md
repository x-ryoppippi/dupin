# dupin-telemetry

The Cloudflare Worker behind `https://telemetry.dupin.app/v1/ping`, which receives the anonymous daily ping described in [Telemetry](https://dupin.app/docs/features/telemetry). The client side lives in `crates/dupin/src/telemetry.rs`; the `Ping` type both sides agree on lives in `crates/dupin-telemetry`, and `src/v1/ping-schema.gen.ts` is generated from it.

The Worker validates the ping against that schema, drops any key it does not know, and writes the six fields to the `dupin_telemetry` [Workers Analytics Engine](https://developers.cloudflare.com/analytics/analytics-engine/) dataset. It stores nothing else: no IP address, headers or Cloudflare request metadata, and Workers Logs are disabled in `void.config.ts`. A request that is not JSON, is larger than 1 KiB or does not match the schema gets a 400 and is not stored; a stored ping gets a 204.

| Column   | Field         | Example                            |
| -------- | ------------- | ---------------------------------- |
| `index1` | `anonymousId` | `4fd4c40cb4140e0e8952ceb2b3502e07` |
| `blob1`  | `event`       | `dupin_daily_active`               |
| `blob2`  | `version`     | `0.4.0`                            |
| `blob3`  | `channel`     | `stable` or `nightly`              |
| `blob4`  | `os`          | `macos 26`                         |
| `blob5`  | `arch`        | `aarch64`                          |

## Daily rollups

Raw pings only live as long as Analytics Engine retains them, so a Cron Trigger at 00:15 UTC recomputes the previous three completed UTC days and writes them to the Worker's KV namespace (binding `ROLLUPS`) as `daily/YYYY-MM-DD.json`. These archives preserve the long-term trend; rerunning a day replaces its file. The public snapshot is rebuilt from the last 90 archive keys in one bulk KV read, overlaid with the current run's results so publication does not depend on immediate read-after-write consistency. A corrupt public snapshot is never used as an input.

```json
{
	"date": "2026-10-21",
	"generatedAt": "2026-10-22T00:15:03.000Z",
	"dailyActiveUsers": 10,
	"weeklyActiveUsers": 25,
	"monthlyActiveUsers": 42,
	"samplingObserved": false,
	"byPlatform": [{ "os": "macos 26", "arch": "aarch64", "users": 7 }],
	"byVersion": [{ "version": "0.4.0", "users": 10 }],
	"byChannel": [{ "channel": "stable", "users": 8 }]
}
```

The weekly and monthly counts are distinct installations over the 7 and 30 days ending that date. They are computed at rollup time because they cannot be rebuilt from daily files once the raw pings expire.

Platform, version and channel queries retain the top 20 groups per day. The limit applies in SQL, before results reach the Worker, and keeps the 90-day public projection well below the KV value-size limit even with high-cardinality valid input. Active-ID totals are queried separately and are not truncated; no synthetic "other" count is produced by summing distinct groups. Publication selects the top 20 groups again from every archive, so an archive written with more stays bounded; it preserves the stored archives, active-ID totals and sampling metadata.

Each rollup records `samplingObserved` from `APPROX_MAX(sampleInterval)` in its six Analytics SQL queries. The installation ID is reused across days and is the dimension being counted; it is not a unique index per event. Sampling intervals are diagnostic metadata, not a multiplier for distinct IDs or an error estimate. The dashboard warns about observed sampling and inconsistent nested-window counts without clamping the returned values.

The three-day lookback recovers recent missed runs; longer outages remain visible as gaps. Rejected scheduled promises appear in Cloudflare's Cron Trigger Past Events status. Request logging stays disabled.

The rollup reads the dataset through an Analytics SQL binding, so it needs no API token. That binding has no local simulation, so the scheduled rollup cannot run in local development; the tests cover the rollup without it.

## Recipes

The Worker is a [Void](https://void.cloud/guide/) app deployed straight to Dupin's Cloudflare account; it does not use the Void platform. `void.config.ts` holds the Worker settings and passes its `cloudflare` block, including the Analytics Engine, Analytics SQL and KV bindings, Workers Cache and the custom domain, through to the generated Cloudflare config. The KV namespace's ID comes from `DUPIN_TELEMETRY_KV_NAMESPACE_ID` when deploying; builds, tests and local development use a local namespace. File conventions map the app:

- `routes/v1/ping.ts` and `routes/v1/stats.ts` are the API, as Hono handlers.
- `crons/rollup.ts` is the daily rollup. Void requires its cron expression to be a literal, so a test checks it against `ROLLUP_CRON`, which the edge cache lifetime uses.
- `pages/index.island.svelte` server-renders the dashboard from the loader in `pages/index.server.ts`. Only the charts and the theme toggle in `src/components/` hydrate as islands; other components keep their styles in their own `<style>` blocks.
- `src/bindings.d.ts` types the bindings so type checking does not depend on Void's generated `.void/` directory.
- `src/v1/ping-schema.gen.ts` is generated; never edit or reformat it. A Rust test fails when it differs from what the `Ping` type produces.

```bash
just telemetry::dev              # run locally with vite dev
just telemetry::test             # Vitest tests
just telemetry::typecheck        # tsc --noEmit
just telemetry::build            # vite build
just telemetry::generate-schema  # regenerate src/v1/ping-schema.gen.ts from the Rust Ping type
```

## Querying

Analytics Engine is queried with SQL over its [HTTP API](https://developers.cloudflare.com/analytics/analytics-engine/sql-api/). Each anonymous ID sends at most one ping per UTC day, so distinct IDs per day are daily active installations:

```sql
SELECT toStartOfDay(timestamp) AS day, count(DISTINCT index1) AS users
FROM dupin_telemetry
WHERE timestamp > NOW() - INTERVAL '30' DAY
GROUP BY day
ORDER BY day
```

Monthly active installations, the platform split, and the channel split:

```sql
SELECT count(DISTINCT index1) AS users
FROM dupin_telemetry
WHERE timestamp > NOW() - INTERVAL '30' DAY;

SELECT blob4 AS os, blob5 AS arch, count(DISTINCT index1) AS users
FROM dupin_telemetry
WHERE timestamp > NOW() - INTERVAL '30' DAY
GROUP BY os, arch
ORDER BY users DESC;

SELECT blob3 AS channel, count(DISTINCT index1) AS users
FROM dupin_telemetry
WHERE timestamp > NOW() - INTERVAL '30' DAY
GROUP BY channel
ORDER BY users DESC;
```

## Public dashboard

`GET /` serves the dashboard, rendered on the server from the same KV snapshot and cached with the same headers and tag as the API, so the cron purge refreshes both. Only the charts and theme toggle ship JavaScript. TanStack Charts and the Dupin logo are bundled as same-origin static assets, and the page uses the system font. It loads no CDN scripts, web fonts or page analytics, and its Content Security Policy allows only its own origin.

`GET /v1/stats` reads a single cached snapshot of the last 90 completed UTC days from the private KV namespace. It validates the stored data and publishes only dates, aggregation timestamps, daily/weekly/monthly active ID counts, sampling metadata, and platform, version and channel aggregates. It never returns installation IDs.

Successful API responses are cached with Workers Cache under the `public-stats` cache tag, and each scheduled run purges that tag after publishing the snapshot. A snapshot that includes the previous UTC day stays at the edge until the next scheduled rollup; one that does not, including the empty state, stays for one minute so a stale read cannot hide a new aggregate. Browsers cache responses for at most five minutes. Missing days remain gaps; invalid data or a KV failure returns an uncacheable 503. Public cache misses read one KV value. A scheduled run uses 18 SQL queries, three archive writes, one bulk read of 90 archive keys and one public snapshot write.

The dashboard distinguishes no data, failed loading, stale aggregates and sampling warnings, and provides a data table and a JSON download alongside its chart. The platform, version and channel breakdowns show the latest day's split as a segmented bar above the ranked list, and the last 30 days as normalised stacked bars. The five largest groups of the latest day keep fixed colours, validated for colour-vision deficiencies in both themes; the rest, including groups beyond the top 20, fold into a grey Other, which is the daily active count minus the named groups because each installation reports one platform, version and channel per day. Every chart has a table view.

There is no public KV namespace or SQL endpoint. Do not add identifiers or raw pings to the public schema. Local development shows the empty state until the local KV namespace contains a public snapshot; `vite dev` prints the local explorer API, whose KV endpoints can write one.

## Origin

This Worker is adapted from [ccusage's telemetry Worker](https://github.com/ccusage/ccusage/tree/main/telemetry), which is available under the MIT licence; its notice is in [`licences/ccusage.txt`](licences/ccusage.txt).

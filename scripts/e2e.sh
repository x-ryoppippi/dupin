#!/usr/bin/env bash
# Drive a built Dupin.app through its accessibility tree with agent-desktop:
# hotkey → panel → type "Calculator" → ↩ → Calculator is running.
# Every agent-desktop reply and Dupin's log land in OUT_DIR for inspection.
# Snapshots contain whatever the panel lists, so run this only on a disposable machine.
set -euo pipefail
app=$(cd "${1:?usage: scripts/e2e.sh path/to/Dupin.app}" && pwd)
out=${OUT_DIR:-target/e2e}
mkdir -p "$out"
out=$(cd "$out" && pwd)

# Runs an agent-desktop command until its reply satisfies a jq predicate; the last reply is
# kept in $out/<name>.json.
until_jq() {
  local name=$1 tries=$2 predicate=$3
  shift 3
  for _ in $(seq "$tries"); do
    agent-desktop "$@" >"$out/$name.json" 2>>"$out/agent-desktop.stderr" || true
    if jq -e "$predicate" "$out/$name.json" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "timed out waiting for $name:" >&2
  cat "$out/$name.json" >&2
  return 1
}

running() { printf '.ok and ([.. | objects | select(.name? == "%s")] | length > 0)' "$1"; }

snap() {
  agent-desktop snapshot --app Dupin >"$out/$1.json" 2>>"$out/agent-desktop.stderr" || true
  screencapture -x "$out/$1.png" 2>/dev/null || true
}

agent-desktop version | tee "$out/version.json"
agent-desktop status | tee "$out/status.json"
agent-desktop permissions | tee "$out/permissions.json"

# ⌘Space belongs to Spotlight on a fresh macOS.
export XDG_CONFIG_HOME=$out/config
mkdir -p "$XDG_CONFIG_HOME/dupin"
echo 'hotkey = "alt+space"' >"$XDG_CONFIG_HOME/dupin/config.toml"

DUPIN_TRACE=1 "$app/Contents/MacOS/dupin" >"$out/dupin.log" 2>&1 &
dupin=$!
trap 'kill "$dupin" 2>/dev/null || true' EXIT

until_jq launched 30 "$(running Dupin)" list-apps
snap launched

agent-desktop press alt+space | tee "$out/press-hotkey.json"
until_jq query-field 10 '.ok and .data.match.ref_id != null' find --app Dupin --role textfield --first
snap summoned
field=$(jq -r '.data.match.ref_id' "$out/query-field.json")

agent-desktop type "$field" Calculator | tee "$out/type.json"
until_jq result 10 '.ok and .data.match.ref_id != null' find --app Dupin --text Calculator --first
snap results

agent-desktop press return | tee "$out/press-return.json"
until_jq opened 15 "$(running Calculator)" list-apps
snap opened

echo "e2e: the panel opened Calculator"
